//! One interface language for the whole app.
//!
//! The shell owns the language: on first run it is the system's (`sys_locale`, which reads
//! `GetUserPreferredUILanguages` on Windows -- where `LANG` is unset -- `CFLocaleCopyPreferredLanguages`
//! on macOS and `LANGUAGE` / `LC_ALL` / `LC_MESSAGES` / `LANG` elsewhere; `REDROB_CANVAS_LOCALE`
//! overrides all of that), and Help > Language sets a user override persisted in
//! `<config dir>/language.json` (`{ "language": "auto" | "<code>" }`). The shell pushes the
//! language into the three editors through their own preferences ([`crate::shell::ShellApp`]),
//! and draws its own strings (mode names, menus, the AI dock, the update texts, About) through
//! [`t`], with Korean here and English as the source.
//!
//! [`LANGUAGES`] lists the languages all three editors support -- the Photo editor has 21, the
//! Vector editor 15, the Layout editor 11 -- so one choice means the same language everywhere.

use std::cell::Cell;
use std::path::PathBuf;

use crate::services;

/// `REDROB_CANVAS_LOCALE=ko-KR`: the system language, for tests and screenshots.
pub const LOCALE_ENV: &str = "REDROB_CANVAS_LOCALE";
/// The preference file under the config directory.
pub const PREF_FILE: &str = "language.json";
/// The preference value that follows the system.
pub const AUTO: &str = "auto";

/// A language all three editors speak.
#[derive(Debug, PartialEq, Eq)]
pub struct LangInfo {
    /// Our code, which is also the Photo and Vector editors' (`interface.language`,
    /// `interfaceLanguage`).
    pub code: &'static str,
    /// The name in that language, for the menu.
    pub name: &'static str,
    /// The Layout editor's code for it (`UiState::language`: `""` is English, `zh` is Simplified).
    pub layout: &'static str,
}

/// The intersection of the three editors' language lists. English first, then by native name.
pub const LANGUAGES: &[LangInfo] = &[
    LangInfo {
        code: "en",
        name: "English",
        layout: "",
    },
    LangInfo {
        code: "de",
        name: "Deutsch",
        layout: "de",
    },
    LangInfo {
        code: "es",
        name: "Español",
        layout: "es",
    },
    LangInfo {
        code: "fr",
        name: "Français",
        layout: "fr",
    },
    LangInfo {
        code: "it",
        name: "Italiano",
        layout: "it",
    },
    LangInfo {
        code: "pt-br",
        name: "Português (Brasil)",
        layout: "pt-br",
    },
    LangInfo {
        code: "uk",
        name: "Українська",
        layout: "uk",
    },
    LangInfo {
        code: "ja",
        name: "日本語",
        layout: "ja",
    },
    LangInfo {
        code: "ko",
        name: "한국어",
        layout: "ko",
    },
    LangInfo {
        code: "zh-hans",
        name: "简体中文",
        layout: "zh",
    },
];

/// A language by our code (case-insensitive); `None` for one not every editor has.
pub fn by_code(code: &str) -> Option<&'static LangInfo> {
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))
}

/// A language by the Layout editor's code (`""` is English).
pub fn by_layout_code(code: &str) -> Option<&'static LangInfo> {
    LANGUAGES.iter().find(|l| l.layout == code)
}

/// English.
pub fn english() -> &'static LangInfo {
    &LANGUAGES[0]
}

/// The language for a locale tag such as `ko_KR.UTF-8`, `ko-KR`, `zh-Hans-CN`, `pt_PT`: most
/// specific first, then the primary subtag; every Chinese tag means Simplified (the only Chinese
/// all three editors have), every Portuguese one Brazilian. `None` for a language not every
/// editor has, and for `C` / `POSIX`, which state no preference (`LC_ALL=C.UTF-8` beside
/// `LANG=ko_KR.UTF-8` is a Korean desktop, so the next tag decides).
pub fn from_tag(tag: &str) -> Option<&'static LangInfo> {
    let base = tag
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let primary = *parts.first()?;
    if matches!(primary, "c" | "posix") {
        return None;
    }
    for n in (1..=parts.len()).rev() {
        if let Some(lang) = by_code(&parts[..n].join("-")) {
            return Some(lang);
        }
    }
    match primary {
        "zh" => by_code("zh-hans"),
        "pt" => by_code("pt-br"),
        _ => None,
    }
}

/// The first supported language among `tags` (most preferred first); English when none is.
pub fn from_tags<S: AsRef<str>>(tags: &[S]) -> &'static LangInfo {
    tags.iter()
        .filter_map(|t| from_tag(t.as_ref()))
        .next()
        .unwrap_or_else(english)
}

/// Redrob Launcher's shared `language.json` (the launcher's `docs/shared-language.md`): the
/// language picked in the launcher's installer or settings, for every Redrob app. It stands in
/// for the system language, so "auto" here follows the launcher; Canvas's own Language choice
/// still wins. `None` when there is no file, or when the launcher only guessed from the system.
fn launcher() -> Option<&'static LangInfo> {
    if std::env::var(LOCALE_ENV).is_ok_and(|t| !t.trim().is_empty()) {
        return None;
    }
    let path = shared_dir()?.join(PREF_FILE);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    // Per-app overrides are keyed by the launcher's app id.
    let own = v
        .get("apps")
        .and_then(|a| a.get("redrob-canvas"))
        .and_then(serde_json::Value::as_str);
    let chosen = v.get("chosen").and_then(serde_json::Value::as_bool) == Some(true);
    let all = v
        .get("language")
        .and_then(serde_json::Value::as_str)
        .filter(|_| chosen);
    own.or(all).and_then(from_tag)
}

/// The launcher's shared directory: `REDROB_SHARED_DIR`, else `%APPDATA%\Redrob\Shared`,
/// `~/Library/Application Support/ai.redrob.shared` or `$XDG_CONFIG_HOME/redrob-shared`.
fn shared_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(d) = env("REDROB_SHARED_DIR") {
        return Some(d);
    }
    if cfg!(windows) {
        env("APPDATA").map(|r| r.join("Redrob").join("Shared"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support/ai.redrob.shared"))
    } else {
        env("XDG_CONFIG_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".config")))
            .map(|r| r.join("redrob-shared"))
    }
}

/// The system's language: `REDROB_CANVAS_LOCALE`, else the OS preferred-languages list.
pub fn system() -> &'static LangInfo {
    if let Ok(tag) = std::env::var(LOCALE_ENV)
        && !tag.trim().is_empty()
    {
        return from_tags(&[tag]);
    }
    // A failure in platform interop must leave the app usable, in English.
    let tags: Vec<String> =
        std::panic::catch_unwind(|| sys_locale::get_locales().take(64).collect())
            .unwrap_or_default();
    from_tags(&tags)
}

// ---------------------------------------------------------------------------------------------
// The preference
// ---------------------------------------------------------------------------------------------

fn pref_path() -> Option<PathBuf> {
    services::config_dir()
        .filter(|_| services::prefs_enabled())
        .map(|d| d.join(PREF_FILE))
}

/// The `language` value of `language.json`: a code or `auto`; `auto` for anything unreadable.
pub fn parse_pref(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("language")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .filter(|p| p.eq_ignore_ascii_case(AUTO) || by_code(p).is_some())
        .unwrap_or_else(|| AUTO.to_owned())
}

fn load_pref() -> String {
    pref_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_pref(&t))
        .unwrap_or_else(|| AUTO.to_owned())
}

fn save_pref(pref: &str) {
    let Some(path) = pref_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::json!({ "language": pref }).to_string() + "\n";
    if let Err(e) = std::fs::write(&path, text) {
        log::warn!("couldn't save {}: {e}", path.display());
    }
}

/// The shell's language setting: the persisted preference and the system language it resolves
/// against.
#[derive(Debug)]
pub struct Language {
    pref: String,
    system: &'static LangInfo,
    /// Whether `set` writes the preference file (not for a setting built in a test).
    persist: bool,
}

impl Language {
    /// From the preference file and the system.
    pub fn load() -> Self {
        Self {
            persist: true,
            ..Self::new(load_pref(), launcher().unwrap_or_else(system))
        }
    }

    /// An explicit setting that is never written to disk (tests).
    pub fn new(pref: String, system: &'static LangInfo) -> Self {
        Self {
            pref,
            system,
            persist: false,
        }
    }

    /// The language the app is drawn in.
    pub fn effective(&self) -> &'static LangInfo {
        if self.pref.eq_ignore_ascii_case(AUTO) {
            self.system
        } else {
            by_code(&self.pref).unwrap_or(self.system)
        }
    }

    /// The preference: `auto` or a code.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn pref(&self) -> &str {
        &self.pref
    }

    /// Whether the setting follows the system.
    pub fn is_auto(&self) -> bool {
        self.pref.eq_ignore_ascii_case(AUTO)
    }

    /// The system language this resolves `auto` against.
    pub fn system(&self) -> &'static LangInfo {
        self.system
    }

    /// Set and persist the preference (`auto` or a code). Returns whether the effective language
    /// changed.
    pub fn set(&mut self, pref: &str) -> bool {
        let before = self.effective();
        self.pref = if pref.eq_ignore_ascii_case(AUTO) || by_code(pref).is_some() {
            pref.to_ascii_lowercase()
        } else {
            AUTO.to_owned()
        };
        if self.persist {
            save_pref(&self.pref);
        }
        before != self.effective()
    }
}

// ---------------------------------------------------------------------------------------------
// The shell's own strings
// ---------------------------------------------------------------------------------------------

thread_local! {
    // The language the shell draws in this frame; set by the shell each frame, read by `t`.
    static CURRENT: Cell<&'static LangInfo> = const { Cell::new(&LANGUAGES[0]) };
}

/// Set the language the shell's own strings are drawn in (once per frame, by the shell).
pub fn set_current(lang: &'static LangInfo) {
    CURRENT.set(lang);
}

/// The language the shell's own strings are drawn in.
pub fn current() -> &'static LangInfo {
    CURRENT.get()
}

/// Translate one of the shell's English strings into the current language; unknown strings and
/// languages without a shell catalog come back unchanged (English).
pub fn t(s: &str) -> &str {
    tr(current().code, s)
}

/// [`t`] for an explicit language code.
pub fn tr<'a>(code: &str, s: &'a str) -> &'a str {
    match code {
        "ko" => KO.iter().find(|(en, _)| *en == s).map_or(s, |(_, ko)| ko),
        _ => s,
    }
}

/// `{name}` placeholders in a translated template.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// [`t`] then [`fmt`].
pub fn tf(s: &str, args: &[(&str, &str)]) -> String {
    fmt(t(s), args)
}

/// The shell's Korean strings (English is the source; every other language keeps English).
const KO: &[(&str, &str)] = &[
    // Modes.
    ("Photo", "사진"),
    ("Vector", "벡터"),
    ("Layout", "레이아웃"),
    // Top bar and menus.
    ("Mode", "모드"),
    ("Help", "도움말"),
    ("Open…", "열기…"),
    (
        "Open a file in the editor its type belongs to",
        "파일을 그 형식에 맞는 편집기에서 엽니다",
    ),
    ("Send to {mode}", "{mode}로 보내기"),
    ("open a document first", "먼저 문서를 여세요"),
    ("this is the active mode", "현재 모드입니다"),
    ("About {app}", "{app} 정보"),
    ("Language", "언어"),
    ("System ({name})", "시스템 ({name})"),
    (
        "Follow the operating system's language",
        "운영 체제의 언어를 따릅니다",
    ),
    (
        "The same language in Photo, Vector and Layout",
        "사진, 벡터, 레이아웃에 같은 언어를 적용합니다",
    ),
    ("Version {version}", "버전 {version}"),
    (
        "MIT OR Apache-2.0. Third-party notices: NOTICE and THIRD_PARTY_NOTICES.md.",
        "MIT 또는 Apache-2.0. 서드파티 고지: NOTICE 및 THIRD_PARTY_NOTICES.md.",
    ),
    ("Open in Redrob Canvas", "Redrob Canvas에서 열기"),
    ("All documents", "모든 문서"),
    // Status line.
    (
        "Opened {name} in {mode}",
        "{name}을(를) {mode}에서 열었습니다",
    ),
    ("Open in {mode} instead", "대신 {mode}에서 열기"),
    (
        "{name}: no editor opens this kind of file",
        "{name}: 이 종류의 파일을 여는 편집기가 없습니다",
    ),
    ("Send to {mode}: {error}", "{mode}로 보내기: {error}"),
    // Exit confirm.
    ("Unsaved changes", "저장하지 않은 변경 사항"),
    (
        "{mode}: {n} document with unsaved changes",
        "{mode}: 저장하지 않은 문서 {n}개",
    ),
    (
        "{mode}: {n} documents with unsaved changes",
        "{mode}: 저장하지 않은 문서 {n}개",
    ),
    (
        "Go to a mode to save its documents, or quit and lose them.",
        "모드로 이동해 문서를 저장하거나, 종료하고 변경 사항을 버리세요.",
    ),
    ("Cancel", "취소"),
    ("Go to {mode}", "{mode}로 이동"),
    ("Quit anyway", "그래도 종료"),
    // AI dock.
    ("Tools", "도구"),
    ("Agent", "에이전트"),
    ("Agent…", "에이전트…"),
    ("Hide AI panel", "AI 패널 숨기기"),
    ("Show AI panel", "AI 패널 표시"),
    (
        "AI tools and agent (toggle the right panel)",
        "AI 도구와 에이전트 (오른쪽 패널 전환)",
    ),
    ("Hide", "숨기기"),
    ("an AI job is running", "AI 작업이 실행 중입니다"),
    // AI dock: the Redrob Console image backend.
    ("Backend", "백엔드"),
    ("Local (IOPaint)", "로컬 (IOPaint)"),
    (
        "Erase, Generative Fill and Expand run on Redrob's image models with your Redrob Console sign-in; usage is billed to your console credit.",
        "지우기, 생성형 채우기, 캔버스 확장이 Redrob Console 로그인으로 Redrob 이미지 모델에서 실행됩니다. 사용량은 콘솔 크레딧에서 차감됩니다.",
    ),
    (
        "Refresh the Redrob Console image models",
        "Redrob Console 이미지 모델 새로 고침",
    ),
    ("Console model", "콘솔 모델"),
    ("Generate image", "이미지 생성"),
    (
        "Describe the image to generate",
        "생성할 이미지를 설명하세요",
    ),
    (
        "A new layer the size of the document, from the prompt (Redrob Console).",
        "프롬프트로 문서 크기의 새 레이어를 만듭니다 (Redrob Console).",
    ),
    (
        "Runs on Redrob Console: no local engine needed.",
        "Redrob Console에서 실행되므로 로컬 엔진이 필요 없습니다.",
    ),
    (
        "Redrob Console image models are not available: sign in, then refresh",
        "Redrob Console 이미지 모델을 쓸 수 없습니다. 로그인한 뒤 새로 고치세요",
    ),
    (
        "pick a Redrob Console model",
        "Redrob Console 모델을 고르세요",
    ),
    (
        "Not enough Redrob credit for this image. Add credit in Redrob Console, then try again.",
        "이 이미지에 필요한 Redrob 크레딧이 부족합니다. Redrob Console에서 크레딧을 충전한 뒤 다시 시도하세요.",
    ),
    (
        "The Redrob image server is busy. Try again in {seconds} s.",
        "Redrob 이미지 서버가 바쁩니다. {seconds}초 뒤에 다시 시도하세요.",
    ),
    (
        "The Redrob image server is busy. Try again in a moment.",
        "Redrob 이미지 서버가 바쁩니다. 잠시 뒤에 다시 시도하세요.",
    ),
    (
        "Sign in to Redrob Console to use its image models.",
        "이미지 모델을 쓰려면 Redrob Console에 로그인하세요.",
    ),
    (
        "Redrob Code is not running, so Redrob Console cannot be reached.",
        "Redrob Code가 실행 중이 아니어서 Redrob Console에 연결할 수 없습니다.",
    ),
    // Updates.
    ("Check for updates…", "업데이트 확인…"),
    ("Check for updates automatically", "자동으로 업데이트 확인"),
    (
        "At start, once a day. Also off with {env}=1.",
        "시작할 때 하루 한 번 확인합니다. {env}=1로도 끌 수 있습니다.",
    ),
    (
        "Updates are managed by Redrob Launcher",
        "업데이트는 Redrob Launcher가 관리합니다",
    ),
    ("Checking…", "확인 중…"),
    ("Up to date (v{version})", "최신 버전입니다 (v{version})"),
    ("v{version} is available", "v{version} 사용 가능"),
    ("Downloading… {pct}%", "다운로드 중… {pct}%"),
    (
        "Downloading v{version}… {pct}%",
        "v{version} 다운로드 중… {pct}%",
    ),
    ("Ready to install", "설치 준비 완료"),
    ("Update check failed", "업데이트 확인 실패"),
    (
        "Update check failed: {error}",
        "업데이트 확인 실패: {error}",
    ),
    (
        "Update check skipped: {error}",
        "업데이트 확인 건너뜀: {error}",
    ),
    ("Update to v{version}", "v{version}로 업데이트"),
    (
        "Download v{version} and verify it ({url})",
        "v{version}을(를) 다운로드하고 검증합니다 ({url})",
    ),
    ("Install and restart", "설치 후 다시 시작"),
    ("Opens {path}", "{path}을(를) 엽니다"),
    ("Last checked {ago}", "마지막 확인: {ago}"),
    ("just now", "방금"),
    ("{n} min ago", "{n}분 전"),
    ("{n} h ago", "{n}시간 전"),
    ("{n} d ago", "{n}일 전"),
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn every_listed_language_is_one_all_three_editors_have() {
        for lang in LANGUAGES {
            assert!(
                redrob_photo_ui_egui::i18n::Lang::from_code(lang.code).is_some(),
                "photo lacks {}",
                lang.code
            );
            assert!(
                redrob_vector_ui_egui::i18n::Lang::from_code(lang.code).is_some(),
                "vector lacks {}",
                lang.code
            );
            assert!(
                redrob_layout_ui_egui::i18n::LANGUAGES
                    .iter()
                    .any(|(c, _)| *c == lang.layout),
                "layout lacks {} ({})",
                lang.code,
                lang.layout
            );
            assert_eq!(by_layout_code(lang.layout), Some(lang));
        }
        assert_eq!(by_code("KO").map(|l| l.name), Some("한국어"));
        assert_eq!(by_code("ru"), None, "the Layout editor has no Russian");
    }

    #[test]
    fn locale_tags_resolve_to_the_common_language_or_english() {
        for (tag, code) in [
            ("ko_KR.UTF-8", "ko"),
            ("ko-KR", "ko"),
            ("ko", "ko"),
            ("ja_JP", "ja"),
            ("zh-Hans-CN", "zh-hans"),
            ("zh_CN.UTF-8", "zh-hans"),
            ("zh-TW", "zh-hans"),
            ("pt_PT", "pt-br"),
            ("pt-BR", "pt-br"),
            ("de-AT", "de"),
            ("fr_CA.UTF-8@euro", "fr"),
            ("uk_UA", "uk"),
            ("en_US.UTF-8", "en"),
        ] {
            assert_eq!(from_tag(tag).map(|l| l.code), Some(code), "{tag}");
        }
        for tag in ["ru_RU", "sv-SE", "", "-", "xx", "C", "C.UTF-8", "POSIX"] {
            assert_eq!(from_tag(tag), None, "{tag:?}");
        }
        // The first supported tag wins; nothing supported means English; `C` is no preference
        // (sys-locale lists `LC_ALL` before `LANG`).
        assert_eq!(from_tags(&["sv-SE", "ko-KR", "en-US"]).code, "ko");
        assert_eq!(from_tags(&["C.UTF-8", "ko-KR"]).code, "ko");
        assert_eq!(from_tags(&["C"]).code, "en");
        assert_eq!(from_tags(&["sv-SE", "ar-SA"]).code, "en");
        assert_eq!(from_tags::<&str>(&[]).code, "en");
    }

    #[test]
    fn the_preference_is_auto_or_a_common_code_and_resolves_against_the_system() {
        assert_eq!(parse_pref(""), "auto");
        assert_eq!(parse_pref("{}"), "auto");
        assert_eq!(parse_pref("{ \"language\": \"ko\" }"), "ko");
        assert_eq!(parse_pref("{ \"language\": \"ru\" }"), "auto", "not common");
        assert_eq!(parse_pref("not json"), "auto");
        let ko = by_code("ko").unwrap();
        let auto = Language::new("auto".into(), ko);
        assert!(auto.is_auto());
        assert_eq!(auto.effective().code, "ko");
        let fixed = Language::new("ja".into(), ko);
        assert!(!fixed.is_auto());
        assert_eq!(fixed.effective().code, "ja");
        assert_eq!(fixed.pref(), "ja");
        assert_eq!(Language::new("ru".into(), ko).effective().code, "ko");
    }

    #[test]
    fn shell_strings_translate_to_korean_and_keep_their_placeholders() {
        assert_eq!(tr("ko", "Photo"), "사진");
        assert_eq!(tr("en", "Photo"), "Photo");
        assert_eq!(
            tr("de", "Photo"),
            "Photo",
            "no German shell catalog: English"
        );
        assert_eq!(tr("ko", "not a shell string"), "not a shell string");
        for (en, ko) in KO {
            let mut placeholders: Vec<&str> = en
                .split('{')
                .skip(1)
                .filter_map(|rest| rest.split('}').next())
                .collect();
            placeholders.sort_unstable();
            for p in &placeholders {
                assert!(ko.contains(&format!("{{{p}}}")), "ko {en:?} lost {{{p}}}");
            }
            assert!(
                KO.iter().filter(|(k, _)| k == en).count() == 1,
                "duplicate {en:?}"
            );
        }
        assert_eq!(
            fmt("Send to {mode}", &[("mode", "Vector")]),
            "Send to Vector"
        );
        set_current(by_code("ko").unwrap());
        assert_eq!(tf("Go to {mode}", &[("mode", "벡터")]), "벡터로 이동");
        set_current(english());
        assert_eq!(t("Go to {mode}"), "Go to {mode}");
    }
}
