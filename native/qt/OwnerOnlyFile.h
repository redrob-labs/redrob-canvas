// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QString>

// A file only the current user may open: the MCP connection token and the console API key.
//
// On Unix that is mode 0600. Windows has no mode bits, and Qt's permission API cannot express
// or report this there: the release smoke read 0x7777 back for a profile file even with NTFS
// lookups on. So on Windows both functions work on the DACL directly.
namespace OwnerOnlyFile {

// Replaces the file's access rules with one rule: the current user, full access. On Windows the
// DACL is also marked protected, so the folder's inherited entries no longer apply.
bool restrictToOwner(const QString &path);

// True when nobody but the current user is granted access. On Windows: every allow entry in the
// DACL names the current user. A missing DACL (which grants everyone everything) is false.
bool isOwnerOnly(const QString &path);

} // namespace OwnerOnlyFile
