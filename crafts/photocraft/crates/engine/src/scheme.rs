//! Script-Fu: a small Scheme interpreter for user scripts and the Script-Fu console. Scripts
//! `(define ...)` procedures, register them with `script-fu-register` (name, menu label, blurb,
//! author, copyright, date, image types, then `SF-*` parameter triples) and `script-fu-menu-
//! register`, and call any engine command as a procedure: a symbol with a dot in it
//! (`(file.new "width" 640 "height" 480)`) or `(call "file.new" '(("width" . 640)))`. Command
//! results come back as Scheme data (objects as association lists, arrays as lists).
//!
//! The language: numbers (f64), strings, symbols, booleans, lists (and dotted pairs from
//! `cons`), lambdas with lexical scope and rest arguments, `define`, `set!`, `if`, `cond`,
//! `case`, `let`, `let*`, `letrec`, `begin`, `and`, `or`, `when`, `unless`, `while`, `do`,
//! `quote`/`'`, quasiquote with `,`/`,@`, and the usual list, string, number, vector-as-list and
//! output builtins. Evaluation is bounded by a step budget and a depth limit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value as Json;

/// A Scheme value.
#[derive(Clone, Debug)]
pub enum Val {
    Nil,
    Bool(bool),
    Num(f64),
    Str(Arc<str>),
    Sym(Arc<str>),
    List(Arc<Vec<Val>>),
    /// An improper pair (`(cons a b)` with a non-list `b`), as `("width" . 640)` in alists.
    Pair(Arc<Val>, Arc<Val>),
    Lambda(Arc<Lambda>),
    Builtin(&'static str),
    /// A procedure standing for an engine command id.
    Command(Arc<str>),
    Unspecified,
}

#[derive(Debug)]
pub struct Lambda {
    pub params: Vec<String>,
    pub rest: Option<String>,
    pub body: Vec<Val>,
    pub env: Env,
    pub name: Option<String>,
}

#[derive(Debug, Default)]
pub struct Frame {
    vars: HashMap<String, Val>,
    parent: Option<Env>,
}

/// A lexical environment (shared, so closures see later definitions).
pub type Env = Arc<Mutex<Frame>>;

fn env_new(parent: Option<Env>) -> Env {
    Arc::new(Mutex::new(Frame { vars: HashMap::new(), parent }))
}

fn env_get(env: &Env, name: &str) -> Option<Val> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let f = e.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(v) = f.vars.get(name) {
            return Some(v.clone());
        }
        cur = f.parent.clone();
    }
    None
}

fn env_set(env: &Env, name: &str, v: Val) -> bool {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let mut f = e.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(slot) = f.vars.get_mut(name) {
            *slot = v;
            return true;
        }
        cur = f.parent.clone();
    }
    false
}

fn env_define(env: &Env, name: &str, v: Val) {
    env.lock().unwrap_or_else(|p| p.into_inner()).vars.insert(name.to_string(), v);
}

/// Errors are strings with the offending form where it helps.
pub type Result<T> = std::result::Result<T, String>;

// ------------------------------------------------------------------------------------- reader

fn tokenize(src: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ';' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '(' | ')' | '\'' | '`' => {
                out.push(c.to_string());
                i += 1;
            }
            ',' => {
                if chars.get(i + 1) == Some(&'@') {
                    out.push(",@".into());
                    i += 2;
                } else {
                    out.push(",".into());
                    i += 1;
                }
            }
            '"' => {
                let mut s = String::from("\"");
                i += 1;
                loop {
                    let Some(&d) = chars.get(i) else { return Err("unterminated string".into()) };
                    i += 1;
                    if d == '\\' {
                        let Some(&e) = chars.get(i) else { return Err("unterminated string".into()) };
                        i += 1;
                        s.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            other => other,
                        });
                    } else if d == '"' {
                        break;
                    } else {
                        s.push(d);
                    }
                }
                out.push(s);
            }
            c if c.is_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '(' | ')' | '\'' | '`' | ',' | ';' | '"') {
                    i += 1;
                }
                out.push(chars[start..i].iter().collect());
            }
        }
    }
    Ok(out)
}

fn atom(tok: &str) -> Val {
    if let Some(s) = tok.strip_prefix('"') {
        return Val::Str(Arc::from(s));
    }
    match tok {
        "#t" | "#true" | "TRUE" => Val::Bool(true),
        "#f" | "#false" | "FALSE" => Val::Bool(false),
        "'()" => Val::Nil,
        _ => match tok.parse::<f64>() {
            Ok(n) if !tok.starts_with('+') || tok.len() > 1 => Val::Num(n),
            _ => Val::Sym(Arc::from(tok)),
        },
    }
}

fn read_from(toks: &[String], at: &mut usize, depth: usize) -> Result<Val> {
    if depth > 512 {
        return Err("expression nested too deeply".into());
    }
    let tok = toks.get(*at).ok_or("unexpected end of input")?.clone();
    *at += 1;
    match tok.as_str() {
        "(" => {
            let mut items = Vec::new();
            loop {
                match toks.get(*at).map(String::as_str) {
                    None => return Err("missing )".into()),
                    Some(")") => {
                        *at += 1;
                        break;
                    }
                    Some(".") if !items.is_empty() => {
                        *at += 1;
                        let tail = read_from(toks, at, depth + 1)?;
                        if toks.get(*at).map(String::as_str) != Some(")") {
                            return Err("bad dotted pair".into());
                        }
                        *at += 1;
                        return Ok(dotted(items, tail));
                    }
                    Some(_) => items.push(read_from(toks, at, depth + 1)?),
                }
            }
            Ok(list(items))
        }
        ")" => Err("unexpected )".into()),
        "'" => Ok(list(vec![sym("quote"), read_from(toks, at, depth + 1)?])),
        "`" => Ok(list(vec![sym("quasiquote"), read_from(toks, at, depth + 1)?])),
        "," => Ok(list(vec![sym("unquote"), read_from(toks, at, depth + 1)?])),
        ",@" => Ok(list(vec![sym("unquote-splicing"), read_from(toks, at, depth + 1)?])),
        _ => Ok(atom(&tok)),
    }
}

/// Reads every top-level form of `src`.
pub fn read_all(src: &str) -> Result<Vec<Val>> {
    let toks = tokenize(src)?;
    let mut at = 0;
    let mut out = Vec::new();
    while at < toks.len() {
        out.push(read_from(&toks, &mut at, 0)?);
    }
    Ok(out)
}

fn sym(s: &str) -> Val {
    Val::Sym(Arc::from(s))
}

fn list(items: Vec<Val>) -> Val {
    if items.is_empty() { Val::Nil } else { Val::List(Arc::new(items)) }
}

fn dotted(mut items: Vec<Val>, tail: Val) -> Val {
    match tail {
        Val::Nil => list(items),
        Val::List(t) => {
            items.extend(t.iter().cloned());
            list(items)
        }
        tail => {
            let last = items.pop().unwrap_or(Val::Nil);
            let pair = Val::Pair(Arc::new(last), Arc::new(tail));
            if items.is_empty() {
                pair
            } else {
                // (a b . c) is kept as a list whose last element is the pair (b . c).
                items.push(pair);
                list(items)
            }
        }
    }
}

// ------------------------------------------------------------------------------------ printing

impl Val {
    fn truthy(&self) -> bool {
        !matches!(self, Val::Bool(false))
    }

    fn items(&self) -> Option<Vec<Val>> {
        match self {
            Val::Nil => Some(Vec::new()),
            Val::List(v) => Some(v.as_ref().clone()),
            _ => None,
        }
    }

    /// Display form (strings unquoted).
    pub fn display(&self) -> String {
        match self {
            Val::Str(s) => s.to_string(),
            other => other.write(),
        }
    }

    /// Written form (strings quoted).
    pub fn write(&self) -> String {
        match self {
            Val::Nil => "()".into(),
            Val::Bool(true) => "#t".into(),
            Val::Bool(false) => "#f".into(),
            Val::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Val::Str(s) => format!("{:?}", s.as_ref()),
            Val::Sym(s) => s.to_string(),
            Val::List(v) => format!("({})", v.iter().map(Val::write).collect::<Vec<_>>().join(" ")),
            Val::Pair(a, b) => format!("({} . {})", a.write(), b.write()),
            Val::Lambda(l) => format!("#<procedure {}>", l.name.as_deref().unwrap_or("lambda")),
            Val::Builtin(n) => format!("#<builtin {n}>"),
            Val::Command(c) => format!("#<command {c}>"),
            Val::Unspecified => String::new(),
        }
    }
}

// -------------------------------------------------------------------------------- JSON bridge

pub fn to_json(v: &Val) -> Json {
    match v {
        Val::Nil | Val::Unspecified => Json::Null,
        Val::Bool(b) => Json::Bool(*b),
        Val::Num(n) => serde_json::Number::from_f64(*n).map_or(Json::Null, Json::Number),
        Val::Str(s) | Val::Sym(s) => Json::String(s.to_string()),
        Val::Pair(a, b) => serde_json::json!({ a.display(): to_json(b) }),
        Val::List(items) => {
            // An association list becomes an object.
            if !items.is_empty()
                && items.iter().all(|i| matches!(i, Val::Pair(..)) || matches!(i, Val::List(l) if l.len() == 2 && matches!(l[0], Val::Str(_) | Val::Sym(_))))
            {
                let mut m = serde_json::Map::new();
                for i in items.iter() {
                    match i {
                        Val::Pair(k, v) => m.insert(k.display(), to_json(v)),
                        Val::List(l) => m.insert(l[0].display(), to_json(&l[1])),
                        _ => None,
                    };
                }
                return Json::Object(m);
            }
            Json::Array(items.iter().map(to_json).collect())
        }
        Val::Lambda(_) | Val::Builtin(_) | Val::Command(_) => Json::String(v.write()),
    }
}

pub fn from_json(j: &Json) -> Val {
    match j {
        Json::Null => Val::Nil,
        Json::Bool(b) => Val::Bool(*b),
        Json::Number(n) => Val::Num(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Val::Str(Arc::from(s.as_str())),
        Json::Array(a) => list(a.iter().map(from_json).collect()),
        Json::Object(o) => list(o.iter().map(|(k, v)| Val::Pair(Arc::new(Val::Str(Arc::from(k.as_str()))), Arc::new(from_json(v)))).collect()),
    }
}

/// Command arguments: one alist/object, or flat `"key" value "key" value` pairs.
fn command_params(args: &[Val]) -> Json {
    if args.len() == 1 {
        let j = to_json(&args[0]);
        return if j.is_object() { j } else { serde_json::json!({}) };
    }
    let mut m = serde_json::Map::new();
    let mut i = 0;
    while i + 1 < args.len() {
        m.insert(args[i].display(), to_json(&args[i + 1]));
        i += 2;
    }
    Json::Object(m)
}

// ------------------------------------------------------------------------------ registration

/// A parameter of a registered script (`SF-ADJUSTMENT "Radius" '(5 1 100 1 10 0 0)` and friends).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptParam {
    pub kind: String,
    pub label: String,
    pub default: Json,
}

/// A script registered with `script-fu-register`.
#[derive(Clone, Debug, PartialEq)]
pub struct Registration {
    pub name: String,
    pub menu: String,
    pub blurb: String,
    pub author: String,
    pub params: Vec<ScriptParam>,
}

/// What a running script can reach: engine commands, through the host's closure.
pub trait Host {
    fn call(&mut self, id: &str, params: Json) -> Result<Json>;
}

/// The interpreter: the global environment, the registrations and the console output.
pub struct Interp {
    pub global: Env,
    pub registered: Vec<Registration>,
    output: String,
    steps: u64,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Interp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interp").field("registered", &self.registered.len()).finish()
    }
}

const MAX_STEPS: u64 = 5_000_000;
const MAX_DEPTH: usize = 800;
const MAX_OUTPUT: usize = 1 << 20;

const BUILTINS: &[&str] = &[
    "+",
    "-",
    "*",
    "/",
    "=",
    "<",
    ">",
    "<=",
    ">=",
    "not",
    "eq?",
    "eqv?",
    "equal?",
    "car",
    "cdr",
    "cons",
    "list",
    "length",
    "append",
    "reverse",
    "list-ref",
    "null?",
    "pair?",
    "list?",
    "number?",
    "string?",
    "symbol?",
    "procedure?",
    "boolean?",
    "zero?",
    "positive?",
    "negative?",
    "abs",
    "min",
    "max",
    "floor",
    "ceiling",
    "round",
    "truncate",
    "sqrt",
    "expt",
    "exp",
    "log",
    "sin",
    "cos",
    "tan",
    "atan",
    "modulo",
    "remainder",
    "quotient",
    "number->string",
    "string->number",
    "string-append",
    "string-length",
    "substring",
    "string-upcase",
    "string-downcase",
    "string=?",
    "string<?",
    "string-split",
    "string-join",
    "string->symbol",
    "symbol->string",
    "display",
    "print",
    "write",
    "newline",
    "apply",
    "map",
    "for-each",
    "filter",
    "assoc",
    "assq",
    "assv",
    "member",
    "memq",
    "cadr",
    "cddr",
    "caddr",
    "caar",
    "cdar",
    "last",
    "list-tail",
    "set-car!",
    "vector",
    "vector-ref",
    "vector-length",
    "make-vector",
    "list->vector",
    "vector->list",
    "call",
    "error",
    "random",
    "script-fu-register",
    "script-fu-menu-register",
    "script-fu-quit",
    "canvas-message",
    "string-search-forward",
    "iota",
    "range",
    "json",
];

impl Interp {
    pub fn new() -> Self {
        let global = env_new(None);
        for b in BUILTINS {
            env_define(&global, b, Val::Builtin(b));
        }
        for (k, v) in [("TRUE", Val::Bool(true)), ("FALSE", Val::Bool(false)), ("nil", Val::Nil), ("pi", Val::Num(std::f64::consts::PI))] {
            env_define(&global, k, v);
        }
        for sf in [
            "SF-ADJUSTMENT",
            "SF-COLOR",
            "SF-TOGGLE",
            "SF-VALUE",
            "SF-STRING",
            "SF-TEXT",
            "SF-FONT",
            "SF-PATTERN",
            "SF-BRUSH",
            "SF-GRADIENT",
            "SF-PALETTE",
            "SF-FILENAME",
            "SF-DIRNAME",
            "SF-OPTION",
            "SF-ENUM",
            "SF-IMAGE",
            "SF-DRAWABLE",
            "SF-LAYER",
            "SF-CHANNEL",
            "SF-VECTORS",
            "SF-DISPLAY",
        ] {
            env_define(&global, sf, Val::Sym(Arc::from(sf)));
        }
        Self { global, registered: Vec::new(), output: String::new(), steps: 0 }
    }

    /// Takes the text `display`/`print` wrote since the last take.
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output)
    }

    fn out(&mut self, s: &str) {
        if self.output.len() < MAX_OUTPUT {
            self.output.push_str(s);
        }
    }

    /// Evaluates every form of `src`; returns the last value.
    pub fn eval_str(&mut self, src: &str, host: &mut dyn Host) -> Result<Val> {
        self.steps = 0;
        let forms = read_all(src)?;
        let mut last = Val::Unspecified;
        for f in forms {
            let g = self.global.clone();
            last = self.eval(&f, &g, host, 0)?;
        }
        Ok(last)
    }

    /// Calls the procedure named `name` with `args`.
    pub fn call_named(&mut self, name: &str, args: Vec<Val>, host: &mut dyn Host) -> Result<Val> {
        self.steps = 0;
        let f = env_get(&self.global, name).ok_or_else(|| format!("no procedure named {name}"))?;
        self.apply(&f, args, host, 0)
    }

    fn eval(&mut self, x: &Val, env: &Env, host: &mut dyn Host, depth: usize) -> Result<Val> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err("the script ran too long (step budget exhausted)".into());
        }
        if depth > MAX_DEPTH {
            return Err("recursion too deep".into());
        }
        match x {
            Val::Sym(s) => {
                if let Some(v) = env_get(env, s) {
                    return Ok(v);
                }
                if s.contains('.') && !s.starts_with('.') {
                    return Ok(Val::Command(s.clone()));
                }
                Err(format!("unbound variable: {s}"))
            }
            Val::List(items) => self.eval_list(items, env, host, depth),
            other => Ok(other.clone()),
        }
    }

    fn eval_list(&mut self, items: &[Val], env: &Env, host: &mut dyn Host, depth: usize) -> Result<Val> {
        let head = &items[0];
        if let Val::Sym(s) = head {
            match s.as_ref() {
                "quote" => return Ok(items.get(1).cloned().unwrap_or(Val::Nil)),
                "quasiquote" => return self.quasi(items.get(1).unwrap_or(&Val::Nil), env, host, depth + 1),
                "if" => {
                    let c = self.eval(items.get(1).ok_or("if: missing condition")?, env, host, depth + 1)?;
                    return if c.truthy() {
                        self.eval(items.get(2).ok_or("if: missing then")?, env, host, depth + 1)
                    } else {
                        match items.get(3) {
                            Some(e) => self.eval(e, env, host, depth + 1),
                            None => Ok(Val::Unspecified),
                        }
                    };
                }
                "define" => {
                    let target = items.get(1).ok_or("define: missing name")?;
                    return match target {
                        Val::Sym(name) => {
                            let v = match items.get(2) {
                                Some(e) => self.eval(e, env, host, depth + 1)?,
                                None => Val::Unspecified,
                            };
                            let v = if let Val::Lambda(l) = &v
                                && l.name.is_none()
                            {
                                Val::Lambda(Arc::new(Lambda {
                                    params: l.params.clone(),
                                    rest: l.rest.clone(),
                                    body: l.body.clone(),
                                    env: l.env.clone(),
                                    name: Some(name.to_string()),
                                }))
                            } else {
                                v
                            };
                            env_define(env, name, v);
                            Ok(Val::Sym(name.clone()))
                        }
                        Val::List(sig) => {
                            // (define (name . params) body...)
                            let Some(Val::Sym(name)) = sig.first() else { return Err("define: bad signature".into()) };
                            let (params, rest) = params_of(&sig[1..])?;
                            let l = Lambda { params, rest, body: items[2..].to_vec(), env: env.clone(), name: Some(name.to_string()) };
                            env_define(env, name, Val::Lambda(Arc::new(l)));
                            Ok(Val::Sym(name.clone()))
                        }
                        Val::Pair(a, b) => {
                            let Val::Sym(name) = a.as_ref() else { return Err("define: bad signature".into()) };
                            let Val::Sym(rest) = b.as_ref() else { return Err("define: bad rest parameter".into()) };
                            let l = Lambda {
                                params: Vec::new(),
                                rest: Some(rest.to_string()),
                                body: items[2..].to_vec(),
                                env: env.clone(),
                                name: Some(name.to_string()),
                            };
                            env_define(env, name, Val::Lambda(Arc::new(l)));
                            Ok(Val::Sym(name.clone()))
                        }
                        _ => Err("define: bad target".into()),
                    };
                }
                "set!" => {
                    let Some(Val::Sym(name)) = items.get(1) else { return Err("set!: missing name".into()) };
                    let v = self.eval(items.get(2).ok_or("set!: missing value")?, env, host, depth + 1)?;
                    if !env_set(env, name, v) {
                        return Err(format!("set!: unbound variable {name}"));
                    }
                    return Ok(Val::Unspecified);
                }
                "lambda" => {
                    let sig = items.get(1).ok_or("lambda: missing parameters")?;
                    let (params, rest) = match sig {
                        Val::Sym(r) => (Vec::new(), Some(r.to_string())),
                        Val::Nil => (Vec::new(), None),
                        Val::List(p) => params_of(p)?,
                        _ => return Err("lambda: bad parameters".into()),
                    };
                    return Ok(Val::Lambda(Arc::new(Lambda { params, rest, body: items[2..].to_vec(), env: env.clone(), name: None })));
                }
                "begin" => return self.eval_body(&items[1..], env, host, depth),
                "let" | "let*" | "letrec" => {
                    let mut idx = 1;
                    let mut loop_name: Option<String> = None;
                    if let Some(Val::Sym(n)) = items.get(1) {
                        loop_name = Some(n.to_string());
                        idx = 2;
                    }
                    let bindings = items.get(idx).and_then(Val::items).ok_or("let: missing bindings")?;
                    let inner = env_new(Some(env.clone()));
                    let sequential = s.as_ref() != "let";
                    let mut names = Vec::new();
                    for b in &bindings {
                        let (name, expr) = match b {
                            Val::List(p) if !p.is_empty() => (p[0].clone(), p.get(1).cloned()),
                            Val::Sym(_) => (b.clone(), None),
                            _ => return Err("let: bad binding".into()),
                        };
                        let Val::Sym(name) = name else { return Err("let: bad binding name".into()) };
                        let v = match expr {
                            Some(e) => self.eval(&e, if sequential { &inner } else { env }, host, depth + 1)?,
                            None => Val::Unspecified,
                        };
                        env_define(&inner, &name, v);
                        names.push(name.to_string());
                    }
                    if let Some(n) = loop_name {
                        // Named let: a procedure over the bindings, called once.
                        let l = Lambda { params: names, rest: None, body: items[idx + 1..].to_vec(), env: inner.clone(), name: Some(n.clone()) };
                        env_define(&inner, &n, Val::Lambda(Arc::new(l)));
                        let args: Vec<Val> = bindings
                            .iter()
                            .filter_map(|b| if let Val::List(p) = b { p.first().cloned() } else { Some(b.clone()) })
                            .map(|nm| env_get(&inner, &nm.display()).unwrap_or(Val::Nil))
                            .collect();
                        let f = env_get(&inner, &n).unwrap_or(Val::Nil);
                        return self.apply(&f, args, host, depth + 1);
                    }
                    return self.eval_body(&items[idx + 1..], &inner, host, depth);
                }
                "cond" => {
                    for clause in &items[1..] {
                        let Some(c) = clause.items() else { return Err("cond: bad clause".into()) };
                        if c.is_empty() {
                            continue;
                        }
                        let test = match &c[0] {
                            Val::Sym(e) if e.as_ref() == "else" => Val::Bool(true),
                            t => self.eval(t, env, host, depth + 1)?,
                        };
                        if test.truthy() {
                            if c.len() == 1 {
                                return Ok(test);
                            }
                            if let Some(Val::Sym(arrow)) = c.get(1)
                                && arrow.as_ref() == "=>"
                            {
                                let f = self.eval(c.get(2).ok_or("cond: => needs a procedure")?, env, host, depth + 1)?;
                                return self.apply(&f, vec![test], host, depth + 1);
                            }
                            return self.eval_body(&c[1..], env, host, depth);
                        }
                    }
                    return Ok(Val::Unspecified);
                }
                "case" => {
                    let key = self.eval(items.get(1).ok_or("case: missing key")?, env, host, depth + 1)?;
                    for clause in &items[2..] {
                        let Some(c) = clause.items() else { continue };
                        let hit = match c.first() {
                            Some(Val::Sym(e)) if e.as_ref() == "else" => true,
                            Some(d) => d.items().unwrap_or_default().iter().any(|v| equal(v, &key)),
                            None => false,
                        };
                        if hit {
                            return self.eval_body(&c[1..], env, host, depth);
                        }
                    }
                    return Ok(Val::Unspecified);
                }
                "and" => {
                    let mut last = Val::Bool(true);
                    for e in &items[1..] {
                        last = self.eval(e, env, host, depth + 1)?;
                        if !last.truthy() {
                            return Ok(last);
                        }
                    }
                    return Ok(last);
                }
                "or" => {
                    for e in &items[1..] {
                        let v = self.eval(e, env, host, depth + 1)?;
                        if v.truthy() {
                            return Ok(v);
                        }
                    }
                    return Ok(Val::Bool(false));
                }
                "when" | "unless" => {
                    let c = self.eval(items.get(1).ok_or("when: missing condition")?, env, host, depth + 1)?;
                    if c.truthy() == (s.as_ref() == "when") {
                        return self.eval_body(&items[2..], env, host, depth);
                    }
                    return Ok(Val::Unspecified);
                }
                "while" => {
                    let mut last = Val::Unspecified;
                    loop {
                        let c = self.eval(items.get(1).ok_or("while: missing condition")?, env, host, depth + 1)?;
                        if !c.truthy() {
                            return Ok(last);
                        }
                        last = self.eval_body(&items[2..], env, host, depth)?;
                    }
                }
                "do" => {
                    // (do ((var init step)...) (test result...) body...)
                    let specs = items.get(1).and_then(Val::items).ok_or("do: missing bindings")?;
                    let inner = env_new(Some(env.clone()));
                    let mut steps = Vec::new();
                    for spec in &specs {
                        let p = spec.items().ok_or("do: bad binding")?;
                        let Some(Val::Sym(name)) = p.first() else { return Err("do: bad binding".into()) };
                        let init = self.eval(p.get(1).ok_or("do: missing init")?, env, host, depth + 1)?;
                        env_define(&inner, name, init);
                        steps.push((name.to_string(), p.get(2).cloned()));
                    }
                    let test = items.get(2).and_then(Val::items).ok_or("do: missing test")?;
                    loop {
                        let t = self.eval(test.first().ok_or("do: empty test")?, &inner, host, depth + 1)?;
                        if t.truthy() {
                            return self.eval_body(&test[1..], &inner, host, depth);
                        }
                        self.eval_body(&items[3..], &inner, host, depth)?;
                        let mut next = Vec::new();
                        for (name, step) in &steps {
                            if let Some(step) = step {
                                next.push((name.clone(), self.eval(step, &inner, host, depth + 1)?));
                            }
                        }
                        for (name, v) in next {
                            env_define(&inner, &name, v);
                        }
                    }
                }
                _ => {}
            }
        }
        let f = self.eval(head, env, host, depth + 1)?;
        let mut args = Vec::with_capacity(items.len() - 1);
        for a in &items[1..] {
            args.push(self.eval(a, env, host, depth + 1)?);
        }
        self.apply(&f, args, host, depth + 1)
    }

    fn eval_body(&mut self, body: &[Val], env: &Env, host: &mut dyn Host, depth: usize) -> Result<Val> {
        let mut last = Val::Unspecified;
        for e in body {
            last = self.eval(e, env, host, depth + 1)?;
        }
        Ok(last)
    }

    fn quasi(&mut self, x: &Val, env: &Env, host: &mut dyn Host, depth: usize) -> Result<Val> {
        match x {
            Val::List(items) => {
                if let Some(Val::Sym(s)) = items.first()
                    && s.as_ref() == "unquote"
                {
                    return self.eval(items.get(1).unwrap_or(&Val::Nil), env, host, depth + 1);
                }
                let mut out = Vec::new();
                for i in items.iter() {
                    if let Val::List(l) = i
                        && let Some(Val::Sym(s)) = l.first()
                        && s.as_ref() == "unquote-splicing"
                    {
                        let v = self.eval(l.get(1).unwrap_or(&Val::Nil), env, host, depth + 1)?;
                        out.extend(v.items().unwrap_or_default());
                        continue;
                    }
                    out.push(self.quasi(i, env, host, depth + 1)?);
                }
                Ok(list(out))
            }
            other => Ok(other.clone()),
        }
    }

    fn apply(&mut self, f: &Val, args: Vec<Val>, host: &mut dyn Host, depth: usize) -> Result<Val> {
        match f {
            Val::Lambda(l) => {
                if args.len() < l.params.len() || (l.rest.is_none() && args.len() > l.params.len()) {
                    return Err(format!("{}: expected {} arguments, got {}", l.name.as_deref().unwrap_or("lambda"), l.params.len(), args.len()));
                }
                let inner = env_new(Some(l.env.clone()));
                let mut it = args.into_iter();
                for p in &l.params {
                    env_define(&inner, p, it.next().unwrap_or(Val::Nil));
                }
                if let Some(r) = &l.rest {
                    env_define(&inner, r, list(it.collect()));
                }
                self.eval_body(&l.body, &inner, host, depth)
            }
            Val::Builtin(name) => self.builtin(name, args, host, depth),
            Val::Command(id) => {
                let r = host.call(id, command_params(&args))?;
                Ok(from_json(&r))
            }
            other => Err(format!("not a procedure: {}", other.write())),
        }
    }

    fn builtin(&mut self, name: &str, args: Vec<Val>, host: &mut dyn Host, depth: usize) -> Result<Val> {
        let num = |v: &Val| -> Result<f64> {
            match v {
                Val::Num(n) => Ok(*n),
                other => Err(format!("expected a number, got {}", other.write())),
            }
        };
        let nums = |args: &[Val]| -> Result<Vec<f64>> { args.iter().map(num).collect() };
        let str_of = |v: &Val| -> Result<String> {
            match v {
                Val::Str(s) | Val::Sym(s) => Ok(s.to_string()),
                other => Err(format!("expected a string, got {}", other.write())),
            }
        };
        let arg = |i: usize| -> Result<&Val> { args.get(i).ok_or_else(|| format!("{name}: missing argument {}", i + 1)) };
        Ok(match name {
            "+" => Val::Num(nums(&args)?.iter().sum()),
            "*" => Val::Num(nums(&args)?.iter().product()),
            "-" => {
                let n = nums(&args)?;
                match n.as_slice() {
                    [] => return Err("-: needs an argument".into()),
                    [x] => Val::Num(-x),
                    [x, rest @ ..] => Val::Num(x - rest.iter().sum::<f64>()),
                }
            }
            "/" => {
                let n = nums(&args)?;
                match n.as_slice() {
                    [] => return Err("/: needs an argument".into()),
                    [x] => Val::Num(1.0 / x),
                    [x, rest @ ..] => Val::Num(rest.iter().fold(*x, |a, b| a / b)),
                }
            }
            "=" | "<" | ">" | "<=" | ">=" => {
                let n = nums(&args)?;
                let ok = n.windows(2).all(|w| match name {
                    "=" => w[0] == w[1],
                    "<" => w[0] < w[1],
                    ">" => w[0] > w[1],
                    "<=" => w[0] <= w[1],
                    _ => w[0] >= w[1],
                });
                Val::Bool(ok)
            }
            "not" => Val::Bool(!arg(0)?.truthy()),
            "eq?" | "eqv?" | "equal?" => Val::Bool(equal(arg(0)?, arg(1)?)),
            "car" => match arg(0)? {
                Val::List(l) => l[0].clone(),
                Val::Pair(a, _) => a.as_ref().clone(),
                other => return Err(format!("car: not a pair: {}", other.write())),
            },
            "cdr" => match arg(0)? {
                Val::List(l) => list(l[1..].to_vec()),
                Val::Pair(_, b) => b.as_ref().clone(),
                other => return Err(format!("cdr: not a pair: {}", other.write())),
            },
            "cadr" => nth(arg(0)?, 1)?,
            "caddr" => nth(arg(0)?, 2)?,
            "cddr" => match arg(0)? {
                Val::List(l) => list(l.get(2..).unwrap_or(&[]).to_vec()),
                other => return Err(format!("cddr: not a list: {}", other.write())),
            },
            "caar" => match nth(arg(0)?, 0)? {
                Val::List(l) => l[0].clone(),
                Val::Pair(a, _) => a.as_ref().clone(),
                other => return Err(format!("caar: not a pair: {}", other.write())),
            },
            "cdar" => match nth(arg(0)?, 0)? {
                Val::List(l) => list(l[1..].to_vec()),
                Val::Pair(_, b) => b.as_ref().clone(),
                other => return Err(format!("cdar: not a pair: {}", other.write())),
            },
            "cons" => {
                let a = arg(0)?.clone();
                match arg(1)? {
                    Val::Nil => list(vec![a]),
                    Val::List(l) => {
                        let mut v = Vec::with_capacity(l.len() + 1);
                        v.push(a);
                        v.extend(l.iter().cloned());
                        list(v)
                    }
                    b => Val::Pair(Arc::new(a), Arc::new(b.clone())),
                }
            }
            "list" | "vector" => list(args),
            "length" | "vector-length" => Val::Num(arg(0)?.items().ok_or("length: not a list")?.len() as f64),
            "append" => {
                let mut out = Vec::new();
                for a in &args {
                    out.extend(a.items().ok_or("append: not a list")?);
                }
                list(out)
            }
            "reverse" => {
                let mut v = arg(0)?.items().ok_or("reverse: not a list")?;
                v.reverse();
                list(v)
            }
            "list-ref" | "vector-ref" => nth(arg(0)?, num(arg(1)?)? as usize)?,
            "list-tail" => {
                let v = arg(0)?.items().ok_or("list-tail: not a list")?;
                let k = (num(arg(1)?)? as usize).min(v.len());
                list(v[k..].to_vec())
            }
            "last" => arg(0)?.items().ok_or("last: not a list")?.last().cloned().unwrap_or(Val::Nil),
            "make-vector" => list(vec![args.get(1).cloned().unwrap_or(Val::Num(0.0)); (num(arg(0)?)? as usize).min(1 << 20)]),
            "list->vector" | "vector->list" => arg(0)?.clone(),
            "set-car!" => return Err("set-car!: lists are immutable here; build a new list".into()),
            "null?" => Val::Bool(matches!(arg(0)?, Val::Nil)),
            "pair?" => Val::Bool(matches!(arg(0)?, Val::List(_) | Val::Pair(..))),
            "list?" => Val::Bool(matches!(arg(0)?, Val::List(_) | Val::Nil)),
            "number?" => Val::Bool(matches!(arg(0)?, Val::Num(_))),
            "string?" => Val::Bool(matches!(arg(0)?, Val::Str(_))),
            "symbol?" => Val::Bool(matches!(arg(0)?, Val::Sym(_))),
            "boolean?" => Val::Bool(matches!(arg(0)?, Val::Bool(_))),
            "procedure?" => Val::Bool(matches!(arg(0)?, Val::Lambda(_) | Val::Builtin(_) | Val::Command(_))),
            "zero?" => Val::Bool(num(arg(0)?)? == 0.0),
            "positive?" => Val::Bool(num(arg(0)?)? > 0.0),
            "negative?" => Val::Bool(num(arg(0)?)? < 0.0),
            "abs" => Val::Num(num(arg(0)?)?.abs()),
            "min" => Val::Num(nums(&args)?.into_iter().fold(f64::INFINITY, f64::min)),
            "max" => Val::Num(nums(&args)?.into_iter().fold(f64::NEG_INFINITY, f64::max)),
            "floor" => Val::Num(num(arg(0)?)?.floor()),
            "ceiling" => Val::Num(num(arg(0)?)?.ceil()),
            "round" => Val::Num(num(arg(0)?)?.round()),
            "truncate" => Val::Num(num(arg(0)?)?.trunc()),
            "sqrt" => Val::Num(num(arg(0)?)?.sqrt()),
            "expt" => Val::Num(num(arg(0)?)?.powf(num(arg(1)?)?)),
            "exp" => Val::Num(num(arg(0)?)?.exp()),
            "log" => Val::Num(num(arg(0)?)?.ln()),
            "sin" => Val::Num(num(arg(0)?)?.sin()),
            "cos" => Val::Num(num(arg(0)?)?.cos()),
            "tan" => Val::Num(num(arg(0)?)?.tan()),
            "atan" => Val::Num(if args.len() > 1 { num(arg(0)?)?.atan2(num(arg(1)?)?) } else { num(arg(0)?)?.atan() }),
            "modulo" => Val::Num(num(arg(0)?)?.rem_euclid(num(arg(1)?)?)),
            "remainder" => Val::Num(num(arg(0)?)? % num(arg(1)?)?),
            "quotient" => Val::Num((num(arg(0)?)? / num(arg(1)?)?).trunc()),
            "random" => {
                let n = num(arg(0)?)?.max(1.0);
                self.steps = self.steps.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                Val::Num(((self.steps >> 33) as f64 % n).floor())
            }
            "number->string" => Val::Str(Arc::from(Val::Num(num(arg(0)?)?).write().as_str())),
            "string->number" => str_of(arg(0)?)?.trim().parse::<f64>().map(Val::Num).unwrap_or(Val::Bool(false)),
            "string-append" => {
                let mut s = String::new();
                for a in &args {
                    s.push_str(&str_of(a)?);
                }
                Val::Str(Arc::from(s.as_str()))
            }
            "string-length" => Val::Num(str_of(arg(0)?)?.chars().count() as f64),
            "substring" => {
                let s: Vec<char> = str_of(arg(0)?)?.chars().collect();
                let a = (num(arg(1)?)? as usize).min(s.len());
                let b = args.get(2).map(num).transpose()?.map_or(s.len(), |b| (b as usize).clamp(a, s.len()));
                Val::Str(Arc::from(s[a..b].iter().collect::<String>().as_str()))
            }
            "string-upcase" => Val::Str(Arc::from(str_of(arg(0)?)?.to_uppercase().as_str())),
            "string-downcase" => Val::Str(Arc::from(str_of(arg(0)?)?.to_lowercase().as_str())),
            "string=?" => Val::Bool(str_of(arg(0)?)? == str_of(arg(1)?)?),
            "string<?" => Val::Bool(str_of(arg(0)?)? < str_of(arg(1)?)?),
            "string-search-forward" => {
                let needle = str_of(arg(0)?)?;
                let hay = str_of(arg(1)?)?;
                hay.find(&needle).map_or(Val::Bool(false), |i| Val::Num(i as f64))
            }
            "string-split" => {
                let s = str_of(arg(0)?)?;
                let sep = args.get(1).map(str_of).transpose()?.unwrap_or_else(|| " ".into());
                list(s.split(sep.as_str()).map(|p| Val::Str(Arc::from(p))).collect())
            }
            "string-join" => {
                let parts: Vec<String> = arg(0)?.items().ok_or("string-join: not a list")?.iter().map(|v| v.display()).collect();
                let sep = args.get(1).map(str_of).transpose()?.unwrap_or_default();
                Val::Str(Arc::from(parts.join(&sep).as_str()))
            }
            "string->symbol" => Val::Sym(Arc::from(str_of(arg(0)?)?.as_str())),
            "symbol->string" => Val::Str(Arc::from(str_of(arg(0)?)?.as_str())),
            "display" | "print" => {
                let s = args.iter().map(Val::display).collect::<Vec<_>>().join(" ");
                self.out(&s);
                if name == "print" {
                    self.out("\n");
                }
                Val::Unspecified
            }
            "write" => {
                let s = args.iter().map(Val::write).collect::<Vec<_>>().join(" ");
                self.out(&s);
                Val::Unspecified
            }
            "newline" => {
                self.out("\n");
                Val::Unspecified
            }
            "canvas-message" => {
                let s = args.iter().map(Val::display).collect::<Vec<_>>().join(" ");
                self.out(&s);
                self.out("\n");
                Val::Unspecified
            }
            "error" => return Err(args.iter().map(Val::display).collect::<Vec<_>>().join(" ")),
            "apply" => {
                let f = arg(0)?.clone();
                let mut a: Vec<Val> = args[1..args.len().saturating_sub(1)].to_vec();
                if args.len() > 1 {
                    a.extend(args[args.len() - 1].items().ok_or("apply: last argument must be a list")?);
                }
                return self.apply(&f, a, host, depth + 1);
            }
            "map" | "for-each" => {
                let f = arg(0)?.clone();
                let lists: Vec<Vec<Val>> = args[1..].iter().map(|l| l.items().ok_or("map: not a list".to_string())).collect::<Result<_>>()?;
                let n = lists.iter().map(Vec::len).min().unwrap_or(0);
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let a: Vec<Val> = lists.iter().map(|l| l[i].clone()).collect();
                    let v = self.apply(&f, a, host, depth + 1)?;
                    if name == "map" {
                        out.push(v);
                    }
                }
                if name == "map" { list(out) } else { Val::Unspecified }
            }
            "filter" => {
                let f = arg(0)?.clone();
                let mut out = Vec::new();
                for v in arg(1)?.items().ok_or("filter: not a list")? {
                    if self.apply(&f, vec![v.clone()], host, depth + 1)?.truthy() {
                        out.push(v);
                    }
                }
                list(out)
            }
            "assoc" | "assq" | "assv" => {
                let key = arg(0)?;
                let found = arg(1)?.items().ok_or("assoc: not a list")?.into_iter().find(|e| match e {
                    Val::Pair(k, _) => equal(k, key),
                    Val::List(l) => l.first().is_some_and(|k| equal(k, key)),
                    _ => false,
                });
                found.unwrap_or(Val::Bool(false))
            }
            "member" | "memq" => {
                let key = arg(0)?;
                let items = arg(1)?.items().ok_or("member: not a list")?;
                match items.iter().position(|v| equal(v, key)) {
                    Some(i) => list(items[i..].to_vec()),
                    None => Val::Bool(false),
                }
            }
            "iota" | "range" => {
                let n = (num(arg(0)?)? as usize).min(1 << 20);
                let start = args.get(1).map(num).transpose()?.unwrap_or(0.0);
                let step = args.get(2).map(num).transpose()?.unwrap_or(1.0);
                list((0..n).map(|i| Val::Num(start + step * i as f64)).collect())
            }
            "json" => Val::Str(Arc::from(to_json(arg(0)?).to_string().as_str())),
            "call" => {
                let id = str_of(arg(0)?)?;
                let r = host.call(&id, command_params(&args[1..]))?;
                from_json(&r)
            }
            "script-fu-register" => {
                // name menu blurb author copyright date image-types [SF-TYPE label default]...
                let name = str_of(arg(0)?)?;
                let menu = args.get(1).map(str_of).transpose()?.unwrap_or_default();
                let blurb = args.get(2).map(str_of).transpose()?.unwrap_or_default();
                let author = args.get(3).map(str_of).transpose()?.unwrap_or_default();
                let mut params = Vec::new();
                let mut i = 7;
                while i + 1 < args.len() {
                    let kind = args[i].display();
                    let label = args[i + 1].display();
                    let default = args.get(i + 2).map(to_json).unwrap_or(Json::Null);
                    params.push(ScriptParam { kind, label, default });
                    i += 3;
                }
                self.registered.retain(|r| r.name != name);
                self.registered.push(Registration { name, menu, blurb, author, params });
                Val::Unspecified
            }
            "script-fu-menu-register" => {
                let name = str_of(arg(0)?)?;
                let menu = str_of(arg(1)?)?;
                if let Some(r) = self.registered.iter_mut().find(|r| r.name == name) {
                    let label = r.menu.rsplit('/').next().unwrap_or(&r.menu).to_string();
                    r.menu = format!("{}/{}", menu.trim_end_matches('/'), label);
                }
                Val::Unspecified
            }
            "script-fu-quit" => Val::Unspecified,
            other => return Err(format!("unknown builtin {other}")),
        })
    }
}

fn nth(v: &Val, i: usize) -> Result<Val> {
    match v {
        Val::List(l) => l.get(i).cloned().ok_or_else(|| format!("index {i} out of range")),
        Val::Pair(a, _) if i == 0 => Ok(a.as_ref().clone()),
        other => Err(format!("not a list: {}", other.write())),
    }
}

fn params_of(items: &[Val]) -> Result<(Vec<String>, Option<String>)> {
    let mut params = Vec::new();
    let mut rest = None;
    for it in items {
        match it {
            Val::Sym(s) => params.push(s.to_string()),
            Val::Pair(a, b) => {
                if let Val::Sym(s) = a.as_ref() {
                    params.push(s.to_string());
                }
                if let Val::Sym(r) = b.as_ref() {
                    rest = Some(r.to_string());
                }
            }
            _ => return Err("bad parameter list".into()),
        }
    }
    Ok((params, rest))
}

fn equal(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Nil, Val::Nil) | (Val::Unspecified, Val::Unspecified) => true,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Num(x), Val::Num(y)) => x == y,
        (Val::Str(x), Val::Str(y)) | (Val::Sym(x), Val::Sym(y)) => x == y,
        (Val::List(x), Val::List(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal(p, q)),
        (Val::Pair(a1, b1), Val::Pair(a2, b2)) => equal(a1, a2) && equal(b1, b2),
        (Val::Builtin(x), Val::Builtin(y)) => x == y,
        (Val::Command(x), Val::Command(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(Vec<(String, Json)>);
    impl Host for Echo {
        fn call(&mut self, id: &str, params: Json) -> Result<Json> {
            self.0.push((id.to_string(), params.clone()));
            Ok(serde_json::json!({"id": id, "ok": true, "params": params}))
        }
    }

    #[test]
    fn core_language() {
        let mut it = Interp::new();
        let mut h = Echo(Vec::new());
        let v = it.eval_str("(define (fact n) (if (<= n 1) 1 (* n (fact (- n 1))))) (fact 10)", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.write(), "3628800");
        let v = it.eval_str("(let loop ((i 0) (acc '())) (if (= i 3) (reverse acc) (loop (+ i 1) (cons i acc))))", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.write(), "(0 1 2)");
        let v = it.eval_str("(map (lambda (x) (* x x)) '(1 2 3))", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.write(), "(1 4 9)");
        let v = it.eval_str("(string-append \"a\" (number->string 2) \"c\")", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.display(), "a2c");
        let v = it.eval_str("`(1 ,(+ 1 1) ,@(list 3 4))", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.write(), "(1 2 3 4)");
        it.eval_str("(display \"hi\") (newline)", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(it.take_output(), "hi\n");
        let v = it.eval_str("(cdr (assoc \"b\" '((\"a\" . 1) (\"b\" . 2))))", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.write(), "2");
        assert!(it.eval_str("(define (f) (f)) (f)", &mut h).is_err(), "runaway recursion is stopped");
    }

    #[test]
    fn commands_and_registration() {
        let mut it = Interp::new();
        let mut h = Echo(Vec::new());
        let v = it.eval_str("(file.new \"width\" 640 \"height\" 480)", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(h.0[0].0, "file.new");
        assert_eq!(h.0[0].1["width"].as_f64(), Some(640.0));
        assert_eq!(nth(&v, 0).map(|p| p.write()).unwrap_or_default(), "(\"id\" . \"file.new\")");
        it.eval_str("(call \"image.flip\" '((\"axis\" . \"horizontal\")))", &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(h.0[1].1["axis"], "horizontal");
        it.eval_str(
            "(define (script-fu-blur-twice image drawable radius) (filter.blur.gaussian \"radius\" radius) (filter.blur.gaussian \"radius\" radius))\n(script-fu-register \"script-fu-blur-twice\" \"<Image>/Filters/Blur/Blur Twice\" \"blur twice\" \"me\" \"c\" \"2026\" \"RGB*\" SF-IMAGE \"Image\" 0 SF-DRAWABLE \"Drawable\" 0 SF-ADJUSTMENT \"Radius\" '(5 1 100 1 10 0 0))\n(script-fu-menu-register \"script-fu-blur-twice\" \"<Image>/Filters/Script-Fu\")",
            &mut h,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(it.registered.len(), 1);
        assert_eq!(it.registered[0].menu, "<Image>/Filters/Script-Fu/Blur Twice");
        assert_eq!(it.registered[0].params.len(), 3);
        it.call_named("script-fu-blur-twice", vec![Val::Num(0.0), Val::Num(0.0), Val::Num(7.0)], &mut h).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(h.0.iter().filter(|c| c.0 == "filter.blur.gaussian").count(), 2);
    }
}
