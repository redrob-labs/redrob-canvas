//! Text › Check Spelling: the document's type checked against the system's hunspell dictionaries
//! (`/usr/share/hunspell`, `/usr/share/myspell`, `/Library/Spelling`, `~/Library/Spelling`,
//! `%APPDATA%\hunspell`; `.dic` word lists with their `.aff` prefix and suffix rules expanded) and
//! `/usr/share/dict/words`, plus a custom dictionary of the user's own words. Commands:
//! `text.spellCheck` lists the misspellings with suggestions, `text.spellReplace` changes one,
//! `text.spellIgnore` skips a word for the session, `text.spellAdd` / `text.spellRemove` edit the
//! custom dictionary, `text.spellLanguages` lists what is installed.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use redrob_vector_doc::NodeKind;
use serde_json::{Value, json};

use super::*;

/// The most words a dictionary expands to (an affix file can multiply a list many times).
const MAX_WORDS: usize = 2_000_000;
/// The most suggestions per misspelling.
const MAX_SUGGESTIONS: usize = 8;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "text.spellCheck",
            "Check Spelling…",
            [],
            None,
            "{language?: a dictionary code such as en_US (default: the first installed English dictionary, else the first installed), ids?: [text objects] (default: every visible, unlocked text object), ignore?: [words] (skipped besides the session's ignored words), limit?: 500} check the document's type → {language, dictionaries: [names], misspellings: [{id, run, start, end (char offsets in the run's text), word, suggestions: [words]}], checked (words), more (the limit was hit)}; without an installed dictionary → an error naming the folders looked in",
            has_doc,
            check
        ),
        cmd!(
            "text.spellReplace",
            "Change Spelling",
            [],
            None,
            "{id, run, start, end, with, all?: false (every occurrence of the word in every text object, as Change All)} replace the misspelled word at the offsets text.spellCheck gave with `with` → {changed}",
            has_doc,
            replace
        ),
        cmd!(
            query "text.spellIgnore",
            "Ignore Spelling",
            [],
            None,
            "{word} skip the word for the rest of the session (Ignore All) → {ignored: [words]}",
            always,
            ignore
        ),
        cmd!(
            query "text.spellAdd",
            "Add to Dictionary",
            [],
            None,
            "{word} add the word to the custom dictionary (Edit Custom Dictionary; saved with the preferences when a folder is set) → {words}",
            always,
            add_word
        ),
        cmd!(
            query "text.spellRemove",
            "Remove from Dictionary",
            [],
            None,
            "{word} remove the word from the custom dictionary → {words}",
            always,
            remove_word
        ),
        cmd!(
            query "text.spellLanguages",
            "Spelling Dictionaries",
            [],
            None,
            "{} → {languages: [{code, name, path}], custom: [words], folders: [paths looked in]}",
            always,
            languages
        ),
    ]
}

/// The user's spelling state: the session's ignored words and the custom dictionary.
#[derive(Default)]
pub struct Spelling {
    pub ignored: HashSet<String>,
    pub custom: Vec<String>,
    /// Where `custom.dic` is kept (none: in memory for the session).
    dir: Option<PathBuf>,
}

impl Spelling {
    /// Keep the custom dictionary in `dir/custom.dic`, reading it now.
    pub fn set_user_dir(&mut self, dir: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(dir.join("custom.dic")) {
            self.custom = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            self.custom.sort();
            self.custom.dedup();
        }
        self.dir = Some(dir);
    }

    fn save(&self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::create_dir_all(dir);
            let text: String = self.custom.iter().map(|w| format!("{w}\n")).collect();
            let _ = std::fs::write(dir.join("custom.dic"), text);
        }
    }
}

/// One installed dictionary.
#[derive(Clone, Debug)]
pub struct DictionaryFile {
    pub code: String,
    pub path: PathBuf,
}

/// The folders dictionaries are looked for in.
fn folders() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if let Ok(extra) = std::env::var("REDROB_VECTOR_DICTIONARIES") {
        v.extend(std::env::split_paths(&extra));
    }
    for f in
        ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/Library/Spelling", "/usr/share/dict"]
    {
        v.push(PathBuf::from(f));
    }
    if let Ok(home) = std::env::var("HOME") {
        v.push(PathBuf::from(&home).join("Library/Spelling"));
        v.push(PathBuf::from(&home).join(".local/share/hunspell"));
    }
    if let Ok(app) = std::env::var("APPDATA") {
        v.push(PathBuf::from(app).join("hunspell"));
    }
    v
}

/// Every `.dic` (and `words`) file installed, by code.
pub fn installed() -> Vec<DictionaryFile> {
    let mut out: Vec<DictionaryFile> = vec![];
    for dir in folders() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let code = if let Some(stem) = name.strip_suffix(".dic") {
                stem.to_string()
            } else if name == "words" && dir.ends_with("dict") {
                "words".to_string()
            } else {
                continue;
            };
            if code.starts_with("hyph_") || code.starts_with("th_") || out.iter().any(|d| d.code == code) {
                continue;
            }
            out.push(DictionaryFile { code, path: p });
        }
    }
    out
}

/// A loaded dictionary: its words (lower-cased and as written).
pub struct Dictionary {
    pub code: String,
    pub names: Vec<String>,
    words: HashSet<String>,
    lower: HashSet<String>,
}

impl Dictionary {
    pub fn has(&self, word: &str) -> bool {
        if self.words.contains(word) || self.lower.contains(&word.to_lowercase()) {
            return true;
        }
        // Capitalised at a sentence start, or all capitals.
        let mut chars = word.chars();
        if let Some(first) = chars.next()
            && first.is_uppercase()
        {
            let rest: String = chars.collect();
            let lowered = format!("{}{}", first.to_lowercase(), rest);
            if self.words.contains(&lowered) {
                return true;
            }
        }
        // Possessive and plural-of-possessive endings.
        for suffix in ["'s", "’s", "'", "’"] {
            if let Some(stem) = word.strip_suffix(suffix)
                && (self.words.contains(stem) || self.lower.contains(&stem.to_lowercase()))
            {
                return true;
            }
        }
        false
    }

    fn insert(&mut self, w: &str) {
        if w.is_empty() || self.words.len() >= MAX_WORDS {
            return;
        }
        self.lower.insert(w.to_lowercase());
        self.words.insert(w.to_string());
    }
}

static CACHE: OnceLock<Mutex<HashMap<String, Arc<Dictionary>>>> = OnceLock::new();

/// The dictionary `code`, loaded once per process.
pub fn load(code: &str) -> std::result::Result<Arc<Dictionary>, String> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(c) = cache.lock()
        && let Some(d) = c.get(code)
    {
        return Ok(d.clone());
    }
    let file = installed().into_iter().find(|d| d.code.eq_ignore_ascii_case(code)).ok_or_else(|| {
        format!(
            "no spelling dictionary `{code}` is installed (looked in {}): install a hunspell dictionary such as hunspell-en-us, or set REDROB_VECTOR_DICTIONARIES to a folder of .dic files",
            folders().iter().map(|f| f.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
        )
    })?;
    let d = Arc::new(read_dictionary(&file)?);
    if let Ok(mut c) = cache.lock() {
        c.insert(code.to_string(), d.clone());
    }
    Ok(d)
}

/// Read `.dic` (with its `.aff`) or a plain word list.
fn read_dictionary(file: &DictionaryFile) -> std::result::Result<Dictionary, String> {
    let bytes = std::fs::read(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
    let aff_path = file.path.with_extension("aff");
    let aff_bytes = std::fs::read(&aff_path).ok();
    // The `.aff` names the encoding (SET ISO8859-1…); most are UTF-8.
    let encoding = aff_bytes
        .as_deref()
        .and_then(|a| String::from_utf8_lossy(a).lines().find_map(|l| l.strip_prefix("SET ").map(|s| s.trim().to_ascii_uppercase())))
        .unwrap_or_else(|| "UTF-8".into());
    let decode = |b: &[u8]| -> String {
        if encoding == "ISO8859-1" || encoding == "ISO-8859-1" || encoding == "ISO8859-15" {
            b.iter().map(|c| *c as char).collect()
        } else {
            String::from_utf8_lossy(b).into_owned()
        }
    };
    let text = decode(&bytes);
    let affixes = aff_bytes.map(|a| parse_affixes(&decode(&a))).unwrap_or_default();
    let mut d = Dictionary { code: file.code.clone(), names: vec![file.path.display().to_string()], words: HashSet::new(), lower: HashSet::new() };
    let mut lines = text.lines();
    // A `.dic` starts with its word count.
    let first = lines.next().unwrap_or("");
    if first.trim().parse::<usize>().is_err() {
        add_entry(&mut d, first, &affixes);
    }
    for line in lines {
        add_entry(&mut d, line, &affixes);
    }
    if d.words.is_empty() {
        return Err(format!("{}: the dictionary is empty", file.path.display()));
    }
    Ok(d)
}

/// `word/FLAGS morph…` → the word and its affixed forms.
fn add_entry(d: &mut Dictionary, line: &str, affixes: &Affixes) {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return;
    }
    let entry = line.split(['\t', ' ']).next().unwrap_or(line);
    // A `\/` is a literal slash.
    let (word, flags) = match entry.find('/') {
        Some(i) if !entry[..i].ends_with('\\') => (&entry[..i], &entry[i + 1..]),
        _ => (entry, ""),
    };
    let word = word.replace("\\/", "/");
    d.insert(&word);
    if flags.is_empty() {
        return;
    }
    let flag_list = affixes.flags(flags);
    let mut forms: Vec<String> = vec![];
    for f in &flag_list {
        for s in affixes.suffixes.get(f).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(w) = s.apply_suffix(&word) {
                forms.push(w);
            }
        }
    }
    for f in &flag_list {
        for p in affixes.prefixes.get(f).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(w) = p.apply_prefix(&word) {
                forms.push(w);
            }
            if p.cross {
                for base in forms.clone() {
                    if let Some(w) = p.apply_prefix(&base) {
                        forms.push(w);
                    }
                }
            }
        }
    }
    for w in forms {
        d.insert(&w);
    }
}

/// One affix rule: strip `strip`, add `add`, where the word matches `cond` at its end (suffix) or
/// start (prefix).
#[derive(Clone, Debug)]
struct Rule {
    strip: String,
    add: String,
    cond: Vec<Cond>,
    cross: bool,
}

#[derive(Clone, Debug)]
enum Cond {
    Any,
    Char(char),
    Set(Vec<char>, bool),
}

impl Rule {
    fn matches_end(&self, w: &str) -> bool {
        let chars: Vec<char> = w.chars().collect();
        if self.cond.len() > chars.len() {
            return false;
        }
        let start = chars.len() - self.cond.len();
        self.cond.iter().enumerate().all(|(i, c)| c.ok(chars[start + i]))
    }
    fn matches_start(&self, w: &str) -> bool {
        let chars: Vec<char> = w.chars().collect();
        if self.cond.len() > chars.len() {
            return false;
        }
        self.cond.iter().enumerate().all(|(i, c)| c.ok(chars[i]))
    }
    fn apply_suffix(&self, w: &str) -> Option<String> {
        if !self.matches_end(w) {
            return None;
        }
        let stem = if self.strip.is_empty() { w } else { w.strip_suffix(self.strip.as_str())? };
        Some(format!("{stem}{}", self.add))
    }
    fn apply_prefix(&self, w: &str) -> Option<String> {
        if !self.matches_start(w) {
            return None;
        }
        let stem = if self.strip.is_empty() { w } else { w.strip_prefix(self.strip.as_str())? };
        Some(format!("{}{stem}", self.add))
    }
}

impl Cond {
    fn ok(&self, c: char) -> bool {
        match self {
            Cond::Any => true,
            Cond::Char(x) => *x == c,
            Cond::Set(set, negated) => set.contains(&c) != *negated,
        }
    }
}

fn parse_cond(s: &str) -> Vec<Cond> {
    let mut out = vec![];
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => out.push(Cond::Any),
            '[' => {
                let negated = chars.peek() == Some(&'^');
                if negated {
                    chars.next();
                }
                let mut set = vec![];
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    set.push(d);
                }
                out.push(Cond::Set(set, negated));
            }
            c => out.push(Cond::Char(c)),
        }
    }
    out
}

#[derive(Default)]
struct Affixes {
    prefixes: HashMap<String, Vec<Rule>>,
    suffixes: HashMap<String, Vec<Rule>>,
    /// FLAG long (two characters per flag) or num (comma-separated numbers); else one character.
    flag_mode: String,
}

impl Affixes {
    fn flags(&self, s: &str) -> Vec<String> {
        match self.flag_mode.as_str() {
            "long" => {
                let c: Vec<char> = s.chars().collect();
                c.chunks(2).map(|p| p.iter().collect()).collect()
            }
            "num" => s.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect(),
            _ => s.chars().map(|c| c.to_string()).collect(),
        }
    }
}

/// `PFX`/`SFX` rules of an `.aff` file.
fn parse_affixes(aff: &str) -> Affixes {
    let mut a = Affixes::default();
    let mut cross: HashMap<(bool, String), bool> = HashMap::new();
    for line in aff.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["FLAG", mode, ..] => a.flag_mode = mode.to_string(),
            [kind @ ("PFX" | "SFX"), flag, rest @ ..] => {
                let is_prefix = *kind == "PFX";
                match rest {
                    // The header: cross product flag and rule count.
                    [yn, count] if count.parse::<usize>().is_ok() && matches!(*yn, "Y" | "N") => {
                        cross.insert((is_prefix, flag.to_string()), *yn == "Y");
                    }
                    [strip, add, cond, ..] => {
                        let strip = if *strip == "0" { String::new() } else { strip.to_string() };
                        // An `add` may carry its own continuation flags after `/`.
                        let add = add.split('/').next().unwrap_or("");
                        let add = if add == "0" { String::new() } else { add.to_string() };
                        let rule = Rule {
                            strip,
                            add,
                            cond: if *cond == "." { vec![] } else { parse_cond(cond) },
                            cross: cross.get(&(is_prefix, flag.to_string())).copied().unwrap_or(false),
                        };
                        let map = if is_prefix { &mut a.prefixes } else { &mut a.suffixes };
                        let rules = map.entry(flag.to_string()).or_default();
                        if rules.len() < 2000 {
                            rules.push(rule);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    a
}

/// Is `c` part of a word?
fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '’' || c == '-'
}

/// The words of `text` with their char offsets.
fn words(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word_char(chars[i]) {
            i += 1;
        }
        let mut end = i;
        // Trailing apostrophes and hyphens aren't part of the word.
        while end > start && !chars[end - 1].is_alphabetic() {
            end -= 1;
        }
        let w: String = chars[start..end].iter().collect();
        out.push((start, end, w));
    }
    out
}

/// Should `w` be checked at all? Numbers, URLs, acronyms and single letters aren't.
fn checkable(w: &str) -> bool {
    let letters = w.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return false;
    }
    // All capitals (acronyms) and mixed-case identifiers (camelCase) pass.
    let uppers = w.chars().filter(|c| c.is_uppercase()).count();
    if uppers == letters || (uppers > 1 && uppers < letters) {
        return false;
    }
    if w.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

/// Suggestions for `word`: dictionary words within edit distance 1 (2 for longer words), the
/// closest first, then the same letters in another case.
fn suggest(d: &Dictionary, word: &str) -> Vec<String> {
    let lower = word.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let n = chars.len();
    let mut cands: Vec<(usize, String)> = vec![];
    let letters: Vec<char> = "abcdefghijklmnopqrstuvwxyz'".chars().chain(extra_letters(&lower)).collect();
    let mut seen: HashSet<String> = HashSet::new();
    fn consider_in(d: &Dictionary, lower: &str, seen: &mut HashSet<String>, w: String, cost: usize, cands: &mut Vec<(usize, String)>) {
        if w != lower && !seen.contains(&w) && d.lower.contains(&w) {
            seen.insert(w.clone());
            cands.push((cost, w));
        }
    }
    let mut consider = |w: String, cost: usize, cands: &mut Vec<(usize, String)>| consider_in(d, &lower, &mut seen, w, cost, cands);
    // Deletions, transpositions, replacements, insertions.
    for i in 0..n {
        let mut v = chars.clone();
        v.remove(i);
        consider(v.iter().collect(), 1, &mut cands);
    }
    for i in 0..n.saturating_sub(1) {
        let mut v = chars.clone();
        v.swap(i, i + 1);
        consider(v.iter().collect(), 1, &mut cands);
    }
    for i in 0..n {
        for l in &letters {
            let mut v = chars.clone();
            v[i] = *l;
            consider(v.iter().collect(), 1, &mut cands);
        }
    }
    for i in 0..=n {
        for l in &letters {
            let mut v = chars.clone();
            v.insert(i, *l);
            consider(v.iter().collect(), 1, &mut cands);
        }
    }
    // A missing space: two words run together.
    for i in 1..n {
        let (a, b): (String, String) = (chars[..i].iter().collect(), chars[i..].iter().collect());
        if d.lower.contains(&a) && d.lower.contains(&b) && !cands.iter().any(|(_, c)| *c == format!("{a} {b}")) {
            cands.push((2, format!("{a} {b}")));
        }
    }
    if cands.len() < 3 && n >= 5 {
        // Edit distance 2 on the shorter neighbourhood: deletions and replacements of a deletion.
        let mut second: Vec<String> = vec![];
        for i in 0..n {
            let mut v = chars.clone();
            v.remove(i);
            for j in 0..v.len() {
                let mut u = v.clone();
                u.remove(j);
                second.push(u.iter().collect());
            }
            for j in 0..v.len() {
                for l in &letters {
                    let mut u = v.clone();
                    u[j] = *l;
                    second.push(u.iter().collect());
                }
            }
        }
        for w in second {
            consider(w, 2, &mut cands);
        }
    }
    cands.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let capitalised = word.chars().next().is_some_and(|c| c.is_uppercase());
    cands
        .into_iter()
        .map(|(_, w)| {
            if capitalised {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => format!("{}{}", f.to_uppercase(), c.as_str()),
                    None => w,
                }
            } else {
                w
            }
        })
        .take(MAX_SUGGESTIONS)
        .collect()
}

/// Letters beyond ASCII the word already uses (so accented variants are tried).
fn extra_letters(w: &str) -> Vec<char> {
    let mut v: Vec<char> = w.chars().filter(|c| !c.is_ascii() && c.is_alphabetic()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The default language: the first English dictionary installed, else the first of any.
fn default_language() -> Option<String> {
    let all = installed();
    all.iter().find(|d| d.code.to_ascii_lowercase().starts_with("en")).or(all.first()).map(|d| d.code.clone())
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spellCheck";
    let language = match str_param(p, "language").filter(|l| !l.trim().is_empty()) {
        Some(l) => l.trim().to_string(),
        None => default_language().ok_or_else(|| {
            bad(
                C,
                format!(
                    "no spelling dictionary is installed (looked in {}): install a hunspell dictionary such as hunspell-en-us, or set REDROB_VECTOR_DICTIONARIES to a folder of .dic files",
                    folders().iter().map(|f| f.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
                ),
            )
        })?,
    };
    let dict = load(&language).map_err(|e| bad(C, e))?;
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(500).clamp(1, 100_000) as usize;
    let mut ignore: HashSet<String> = s.spelling.ignored.iter().map(|w| w.to_lowercase()).collect();
    ignore.extend(s.spelling.custom.iter().map(|w| w.to_lowercase()));
    if let Some(Value::Array(a)) = p.get("ignore") {
        ignore.extend(a.iter().filter_map(Value::as_str).map(str::to_lowercase));
    }
    let st = s.doc()?;
    let ids: Vec<NodeId> = match ids_param(p, "ids") {
        Some(v) => super::typemenu::text_ids(&st.doc, &v),
        None => {
            let mut v = vec![];
            for l in st.doc.layers.iter().filter(|l| l.visible && !l.locked) {
                l.walk(&mut |n| {
                    if matches!(n.kind, NodeKind::Text(_)) && n.visible && !n.locked {
                        v.push(n.id);
                    }
                });
            }
            v
        }
    };
    let mut out = vec![];
    let mut checked = 0usize;
    let mut more = false;
    'outer: for id in ids {
        let Some(NodeKind::Text(t)) = st.doc.node(id).map(|n| &n.kind) else { continue };
        for (ri, run) in t.runs.iter().enumerate() {
            if run.inline.is_some() {
                continue;
            }
            for (start, end, w) in words(&run.text) {
                if !checkable(&w) {
                    continue;
                }
                checked += 1;
                if ignore.contains(&w.to_lowercase()) || dict.has(&w) {
                    continue;
                }
                if out.len() >= limit {
                    more = true;
                    break 'outer;
                }
                out.push(json!({
                    "id": id.0,
                    "run": ri,
                    "start": start,
                    "end": end,
                    "word": w,
                    "suggestions": suggest(&dict, &w),
                }));
            }
        }
    }
    Ok(json!({ "language": dict.code, "dictionaries": dict.names, "misspellings": out, "checked": checked, "more": more }))
}

fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spellReplace";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    let run = p.get("run").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `run`"))? as usize;
    let start = p.get("start").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `start`"))? as usize;
    let end = p.get("end").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `end`"))? as usize;
    let with = str_param(p, "with").ok_or_else(|| bad(C, "missing `with`"))?.to_string();
    let all = p.get("all").and_then(Value::as_bool).unwrap_or(false);
    if end <= start {
        return Err(bad(C, "`end` must be after `start`"));
    }
    // The word as it stands, for Change All.
    let word = {
        let st = s.doc()?;
        let Some(NodeKind::Text(t)) = st.doc.node(id).map(|n| &n.kind) else { return Err(bad(C, "not a text object")) };
        let r = t.runs.get(run).ok_or_else(|| bad(C, "no such run"))?;
        let chars: Vec<char> = r.text.chars().collect();
        if end > chars.len() {
            return Err(bad(C, "the offsets are past the end of the run (the text changed: check again)"));
        }
        chars[start..end].iter().collect::<String>()
    };
    let changed = super::overprint::edit_counted(s, "Change Spelling", |d| {
        let mut changed = 0;
        let ids: Vec<NodeId> = if all {
            let mut v = vec![];
            d.walk(|n| {
                if matches!(n.kind, NodeKind::Text(_)) {
                    v.push(n.id);
                }
            });
            v
        } else {
            vec![id]
        };
        for tid in ids {
            let Some(NodeKind::Text(t)) = d.node_mut(tid).map(|n| &mut n.kind) else { continue };
            let mut any = false;
            for (ri, r) in t.runs.iter_mut().enumerate() {
                if r.inline.is_some() {
                    continue;
                }
                if all {
                    let mut out = String::new();
                    let mut last = 0;
                    for (a, b, w) in words(&r.text) {
                        if w == word {
                            let chars: Vec<char> = r.text.chars().collect();
                            out.push_str(&chars[last..a].iter().collect::<String>());
                            out.push_str(&with);
                            last = b;
                        }
                    }
                    if last > 0 {
                        let chars: Vec<char> = r.text.chars().collect();
                        out.push_str(&chars[last..].iter().collect::<String>());
                        r.text = out;
                        any = true;
                    }
                } else if ri == run {
                    let chars: Vec<char> = r.text.chars().collect();
                    if end <= chars.len() {
                        let mut out: String = chars[..start].iter().collect();
                        out.push_str(&with);
                        out.extend(chars[end..].iter());
                        r.text = out;
                        any = true;
                    }
                }
            }
            if any {
                super::typecmd::refresh_bounds(t);
                changed += 1;
            }
        }
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

fn ignore(s: &mut Session, p: &Value) -> Result<Value> {
    let word = str_param(p, "word").map(str::trim).filter(|w| !w.is_empty()).ok_or_else(|| bad("text.spellIgnore", "missing `word`"))?;
    s.spelling.ignored.insert(word.to_string());
    let mut v: Vec<&String> = s.spelling.ignored.iter().collect();
    v.sort();
    Ok(json!({ "ignored": v }))
}

fn add_word(s: &mut Session, p: &Value) -> Result<Value> {
    let word = str_param(p, "word").map(str::trim).filter(|w| !w.is_empty()).ok_or_else(|| bad("text.spellAdd", "missing `word`"))?;
    if !s.spelling.custom.iter().any(|w| w == word) {
        s.spelling.custom.push(word.to_string());
        s.spelling.custom.sort();
        s.spelling.save();
    }
    Ok(json!({ "words": s.spelling.custom }))
}

fn remove_word(s: &mut Session, p: &Value) -> Result<Value> {
    let word = str_param(p, "word").map(str::trim).filter(|w| !w.is_empty()).ok_or_else(|| bad("text.spellRemove", "missing `word`"))?;
    s.spelling.custom.retain(|w| w != word);
    s.spelling.save();
    Ok(json!({ "words": s.spelling.custom }))
}

fn languages(s: &mut Session, _: &Value) -> Result<Value> {
    let langs: Vec<Value> = installed()
        .into_iter()
        .map(|d| {
            let name = language_name(&d.code);
            json!({ "code": d.code, "name": name, "path": d.path.display().to_string() })
        })
        .collect();
    Ok(json!({
        "languages": langs,
        "custom": s.spelling.custom,
        "folders": folders().iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
    }))
}

/// A readable name for a dictionary code (`en_US` → English (US)).
fn language_name(code: &str) -> String {
    let (lang, region) = code.split_once(['_', '-']).map(|(l, r)| (l, Some(r))).unwrap_or((code, None));
    let name = match lang.to_ascii_lowercase().as_str() {
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "da" => "Danish",
        "nb" | "nn" | "no" => "Norwegian",
        "fi" => "Finnish",
        "pl" => "Polish",
        "cs" => "Czech",
        "sk" => "Slovak",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "el" => "Greek",
        "tr" => "Turkish",
        "ca" => "Catalan",
        "gl" => "Galician",
        "eu" => "Basque",
        "hr" => "Croatian",
        "sl" => "Slovenian",
        "sr" => "Serbian",
        "bg" => "Bulgarian",
        "et" => "Estonian",
        "lv" => "Latvian",
        "lt" => "Lithuanian",
        "is" => "Icelandic",
        "ga" => "Irish",
        "af" => "Afrikaans",
        "id" => "Indonesian",
        "vi" => "Vietnamese",
        "words" => "System word list",
        _ => "",
    };
    match (name.is_empty(), region) {
        (true, _) => code.to_string(),
        (false, Some(r)) => format!("{name} ({})", r.to_ascii_uppercase()),
        (false, None) => name.to_string(),
    }
}
