// SPDX-License-Identifier: GPL-3.0-or-later

//! Cooperative cancellation of a long command (P8b).
//!
//! A filter runs inside `Editor::execute`, which the FFI calls with the editor locked. A cancel
//! request therefore cannot go through the editor: it would wait for the very lock the running
//! filter holds. Instead the caller owns a [`CancelToken`] OUTSIDE the lock, installs it on the
//! worker thread with [`with_cancel`] around the call, and flips it from any other thread.
//!
//! Long loops call [`checkpoint`], which returns [`CoreError::Cancelled`] once the token is set.
//! The command path is transactional (it edits a clone of the document and commits only on
//! success), so a cancelled filter leaves the document and the history exactly as they were.
//! A filter with no checkpoint in its loop still runs to the end, and is then discarded by the
//! checkpoint `apply_filter` takes before it commits: cancel always wins, it is only the CPU time
//! that differs.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{CoreError, Result};

/// A shareable "please stop" flag. Cloning shares the flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the command running under this token to stop at its next checkpoint.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Clears a request, so the next command does not inherit a cancel aimed at the last one.
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

thread_local! {
    static CURRENT: RefCell<Option<CancelToken>> = const { RefCell::new(None) };
}

/// Runs `body` with `token` as this thread's cancel token, restoring the previous one afterwards
/// (also on panic-free early return), so nested or later calls are not affected.
pub fn with_cancel<T>(token: &CancelToken, body: impl FnOnce() -> T) -> T {
    struct Restore(Option<CancelToken>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CURRENT.with(|slot| *slot.borrow_mut() = previous);
        }
    }
    let previous = CURRENT.with(|slot| slot.borrow_mut().replace(token.clone()));
    let _restore = Restore(previous);
    body()
}

/// `Err(Cancelled)` once this thread's token has been cancelled; `Ok` when there is no token.
pub(crate) fn checkpoint() -> Result<()> {
    let cancelled = CURRENT.with(|slot| slot.borrow().as_ref().is_some_and(CancelToken::is_cancelled));
    if cancelled {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}
