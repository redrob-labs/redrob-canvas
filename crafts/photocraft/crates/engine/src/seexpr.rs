//! Generator scripts (`scripts.seexpr.*`, `fill.seexpr`): a small expression language that
//! computes a colour per pixel, saved as named resources and rendered into a layer (the fill
//! layer's generator). The language is a subset of the SeExpr idiom as the reference's manual
//! describes it: `u`, `v` (0..1 across the image), `x`, `y`, `w`, `h` (pixels), numbers,
//! `[r, g, b]` vectors, `+ - * / % ^`, comparisons and `? :`, `$name = expr;` assignments before
//! the final expression, and the functions `abs sin cos tan sqrt exp log floor ceil fract pow
//! min max clamp mix lerp smoothstep step length dist noise cnoise hash rgb hsv`. Scalars
//! broadcast over vectors. Nothing of the reference's implementation is read; this is a
//! textbook recursive-descent evaluator.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, has_paintable};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Longest script source accepted.
pub const MAX_SOURCE: usize = 64 * 1024;
/// Most saved scripts.
pub const MAX_SCRIPTS: usize = 1000;

/// A saved generator script.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SeExprScript {
    pub name: String,
    pub source: String,
    pub tags: Vec<String>,
}

/// A 3-vector value (scalars are stored with all three the same and `scalar` set).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Val {
    v: [f32; 3],
    scalar: bool,
}

impl Val {
    fn s(x: f32) -> Self {
        Val { v: [x, x, x], scalar: true }
    }
    fn vec(v: [f32; 3]) -> Self {
        Val { v, scalar: false }
    }
    fn map(self, f: impl Fn(f32) -> f32) -> Self {
        Val { v: [f(self.v[0]), f(self.v[1]), f(self.v[2])], scalar: self.scalar }
    }
    fn zip(self, o: Val, f: impl Fn(f32, f32) -> f32) -> Self {
        Val { v: [f(self.v[0], o.v[0]), f(self.v[1], o.v[1]), f(self.v[2], o.v[2])], scalar: self.scalar && o.scalar }
    }
    fn x(self) -> f32 {
        self.v[0]
    }
}

/// Parsed expression tree.
#[derive(Clone, Debug)]
enum Expr {
    Num(f32),
    Var(String),
    Vec(Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// A compiled script: assignments then the result expression.
#[derive(Clone, Debug)]
pub struct Program {
    assigns: Vec<(String, Expr)>,
    result: Expr,
}

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        loop {
            while self.at < self.src.len() && self.src[self.at].is_ascii_whitespace() {
                self.at += 1;
            }
            // `#` and `//` comments to the end of the line.
            if self.src.get(self.at) == Some(&b'#') || (self.src.get(self.at) == Some(&b'/') && self.src.get(self.at + 1) == Some(&b'/')) {
                while self.at < self.src.len() && self.src[self.at] != b'\n' {
                    self.at += 1;
                }
                continue;
            }
            break;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.at).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn eat_str(&mut self, s: &str) -> bool {
        self.skip_ws();
        if self.src[self.at..].starts_with(s.as_bytes()) {
            self.at += s.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: u8) -> std::result::Result<(), String> {
        if self.eat(c) { Ok(()) } else { Err(format!("expected `{}` at {}", c as char, self.at)) }
    }
    fn ident(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.at;
        while self.at < self.src.len() && (self.src[self.at].is_ascii_alphanumeric() || self.src[self.at] == b'_') {
            self.at += 1;
        }
        if self.at == start || self.src[start].is_ascii_digit() {
            self.at = start;
            return None;
        }
        Some(String::from_utf8_lossy(&self.src[start..self.at]).into_owned())
    }
    fn number(&mut self) -> Option<f32> {
        self.skip_ws();
        let start = self.at;
        while self.at < self.src.len() && (self.src[self.at].is_ascii_digit() || self.src[self.at] == b'.') {
            self.at += 1;
        }
        if self.at < self.src.len() && (self.src[self.at] == b'e' || self.src[self.at] == b'E') {
            let save = self.at;
            self.at += 1;
            if self.at < self.src.len() && (self.src[self.at] == b'-' || self.src[self.at] == b'+') {
                self.at += 1;
            }
            let d = self.at;
            while self.at < self.src.len() && self.src[self.at].is_ascii_digit() {
                self.at += 1;
            }
            if self.at == d {
                self.at = save;
            }
        }
        if self.at == start {
            return None;
        }
        let text = std::str::from_utf8(&self.src[start..self.at]).ok()?;
        match text.parse::<f32>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.at = start;
                None
            }
        }
    }

    fn program(&mut self) -> std::result::Result<Program, String> {
        let mut assigns = Vec::new();
        loop {
            self.skip_ws();
            if self.src.get(self.at) == Some(&b'$') {
                let save = self.at;
                self.at += 1;
                let name = self.ident().ok_or("expected a variable name after `$`")?;
                if self.eat(b'=') && self.src.get(self.at) != Some(&b'=') {
                    let e = self.expr()?;
                    self.expect(b';')?;
                    assigns.push((name, e));
                    continue;
                }
                self.at = save;
            }
            break;
        }
        let result = self.expr()?;
        let _ = self.eat(b';');
        self.skip_ws();
        if self.at < self.src.len() {
            return Err(format!("unexpected `{}` at {}", self.src[self.at] as char, self.at));
        }
        Ok(Program { assigns, result })
    }

    fn expr(&mut self) -> std::result::Result<Expr, String> {
        let c = self.or()?;
        if self.eat(b'?') {
            let a = self.expr()?;
            self.expect(b':')?;
            let b = self.expr()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }
    fn or(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.and()?;
        while self.eat_str("||") {
            let b = self.and()?;
            a = Expr::Or(Box::new(a), Box::new(b));
        }
        Ok(a)
    }
    fn and(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.cmp()?;
        while self.eat_str("&&") {
            let b = self.cmp()?;
            a = Expr::And(Box::new(a), Box::new(b));
        }
        Ok(a)
    }
    fn cmp(&mut self) -> std::result::Result<Expr, String> {
        let a = self.add()?;
        for op in ["<=", ">=", "==", "!=", "<", ">"] {
            if self.eat_str(op) {
                let b = self.add()?;
                return Ok(Expr::Cmp(op, Box::new(a), Box::new(b)));
            }
        }
        Ok(a)
    }
    fn add(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.mul()?;
        loop {
            if self.eat(b'+') {
                let b = self.mul()?;
                a = Expr::Bin('+', Box::new(a), Box::new(b));
            } else if self.peek() == Some(b'-') && self.src.get(self.at + 1) != Some(&b'>') {
                self.at += 1;
                let b = self.mul()?;
                a = Expr::Bin('-', Box::new(a), Box::new(b));
            } else {
                return Ok(a);
            }
        }
    }
    fn mul(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.pow()?;
        loop {
            let op = match self.peek() {
                Some(b'*') => '*',
                Some(b'/') if self.src.get(self.at + 1) != Some(&b'/') => '/',
                Some(b'%') => '%',
                _ => return Ok(a),
            };
            self.at += 1;
            let b = self.pow()?;
            a = Expr::Bin(op, Box::new(a), Box::new(b));
        }
    }
    fn pow(&mut self) -> std::result::Result<Expr, String> {
        let a = self.unary()?;
        if self.eat(b'^') {
            let b = self.pow()?;
            return Ok(Expr::Bin('^', Box::new(a), Box::new(b)));
        }
        Ok(a)
    }
    fn unary(&mut self) -> std::result::Result<Expr, String> {
        if self.eat(b'-') {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat(b'+') {
            return self.unary();
        }
        if self.peek() == Some(b'!') && self.src.get(self.at + 1) != Some(&b'=') {
            self.at += 1;
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.atom()
    }
    fn atom(&mut self) -> std::result::Result<Expr, String> {
        if self.eat(b'(') {
            let e = self.expr()?;
            self.expect(b')')?;
            return Ok(e);
        }
        if self.eat(b'[') {
            let mut items = vec![self.expr()?];
            while self.eat(b',') {
                items.push(self.expr()?);
            }
            self.expect(b']')?;
            if items.len() != 3 && items.len() != 1 {
                return Err("a vector has 3 components".into());
            }
            return Ok(Expr::Vec(items));
        }
        if let Some(n) = self.number() {
            return Ok(Expr::Num(n));
        }
        let dollar = self.eat(b'$');
        let Some(name) = self.ident() else {
            return Err(format!("unexpected input at {}", self.at));
        };
        if !dollar && self.eat(b'(') {
            let mut args = Vec::new();
            if !self.eat(b')') {
                args.push(self.expr()?);
                while self.eat(b',') {
                    args.push(self.expr()?);
                }
                self.expect(b')')?;
            }
            return Ok(Expr::Call(name, args));
        }
        Ok(Expr::Var(name))
    }
}

/// Compile a script.
pub fn compile(src: &str) -> std::result::Result<Program, String> {
    if src.len() > MAX_SOURCE {
        return Err("the script is too long".into());
    }
    let mut p = Parser { src: src.as_bytes(), at: 0 };
    p.program()
}

fn hash01(x: f32, y: f32, z: f32) -> f32 {
    let h =
        redrob_photo_paint::rng::mix64((x.to_bits() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((y.to_bits() as u64) << 21) ^ ((z.to_bits() as u64) << 42));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth value noise in 0..1 over a lattice.
fn noise2(x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let n = |i: f32, j: f32| hash01(x0 + i, y0 + j, 7.0);
    let a = n(0.0, 0.0) + (n(1.0, 0.0) - n(0.0, 0.0)) * sx;
    let b = n(0.0, 1.0) + (n(1.0, 1.0) - n(0.0, 1.0)) * sx;
    a + (b - a) * sy
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

struct Env<'a> {
    vars: &'a [(String, Val)],
    locals: Vec<(String, Val)>,
}

impl Env<'_> {
    fn get(&self, name: &str) -> Option<Val> {
        self.locals.iter().rev().find(|(n, _)| n == name).or_else(|| self.vars.iter().find(|(n, _)| n == name)).map(|(_, v)| *v)
    }
}

fn eval(e: &Expr, env: &Env) -> std::result::Result<Val, String> {
    Ok(match e {
        Expr::Num(n) => Val::s(*n),
        Expr::Var(n) => env.get(n).ok_or_else(|| format!("unknown variable `{n}`"))?,
        Expr::Vec(items) => {
            if items.len() == 1 {
                let a = eval(&items[0], env)?;
                Val::vec(a.v)
            } else {
                Val::vec([eval(&items[0], env)?.x(), eval(&items[1], env)?.x(), eval(&items[2], env)?.x()])
            }
        }
        Expr::Neg(a) => eval(a, env)?.map(|x| -x),
        Expr::Not(a) => eval(a, env)?.map(|x| if x == 0.0 { 1.0 } else { 0.0 }),
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval(a, env)?, eval(b, env)?);
            match op {
                '+' => a.zip(b, |x, y| x + y),
                '-' => a.zip(b, |x, y| x - y),
                '*' => a.zip(b, |x, y| x * y),
                '/' => a.zip(b, |x, y| if y == 0.0 { 0.0 } else { x / y }),
                '%' => a.zip(b, |x, y| if y == 0.0 { 0.0 } else { x.rem_euclid(y) }),
                '^' => a.zip(b, |x, y| x.powf(y)),
                _ => return Err(format!("bad operator {op}")),
            }
        }
        Expr::Cmp(op, a, b) => {
            let (a, b) = (eval(a, env)?.x(), eval(b, env)?.x());
            let r = match *op {
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                ">=" => a >= b,
                "==" => a == b,
                _ => a != b,
            };
            Val::s(if r { 1.0 } else { 0.0 })
        }
        Expr::And(a, b) => Val::s(if eval(a, env)?.x() != 0.0 && eval(b, env)?.x() != 0.0 { 1.0 } else { 0.0 }),
        Expr::Or(a, b) => Val::s(if eval(a, env)?.x() != 0.0 || eval(b, env)?.x() != 0.0 { 1.0 } else { 0.0 }),
        Expr::Cond(c, a, b) => {
            if eval(c, env)?.x() != 0.0 {
                eval(a, env)?
            } else {
                eval(b, env)?
            }
        }
        Expr::Call(name, args) => {
            let a: Vec<Val> = args.iter().map(|x| eval(x, env)).collect::<std::result::Result<_, _>>()?;
            let need = |n: usize| if a.len() < n { Err(format!("`{name}` needs {n} arguments")) } else { Ok(()) };
            let f1 = |f: fn(f32) -> f32| -> std::result::Result<Val, String> {
                need(1)?;
                Ok(a[0].map(f))
            };
            match name.as_str() {
                "abs" => f1(f32::abs)?,
                "sin" => f1(f32::sin)?,
                "cos" => f1(f32::cos)?,
                "tan" => f1(f32::tan)?,
                "sqrt" => f1(|x| x.max(0.0).sqrt())?,
                "exp" => f1(f32::exp)?,
                "log" => f1(|x| if x > 0.0 { x.ln() } else { 0.0 })?,
                "floor" => f1(f32::floor)?,
                "ceil" => f1(f32::ceil)?,
                "round" => f1(f32::round)?,
                "fract" => f1(f32::fract)?,
                "sign" => f1(f32::signum)?,
                "pow" => {
                    need(2)?;
                    a[0].zip(a[1], f32::powf)
                }
                "min" => {
                    need(2)?;
                    a[1..].iter().fold(a[0], |m, v| m.zip(*v, f32::min))
                }
                "max" => {
                    need(2)?;
                    a[1..].iter().fold(a[0], |m, v| m.zip(*v, f32::max))
                }
                "clamp" => {
                    need(3)?;
                    a[0].zip(a[1], f32::max).zip(a[2], f32::min)
                }
                "mix" | "lerp" => {
                    need(3)?;
                    let t = a[2];
                    let d = a[1].zip(a[0], |x, y| x - y);
                    a[0].zip(d.zip(t, |x, y| x * y), |x, y| x + y)
                }
                "step" => {
                    need(2)?;
                    a[1].zip(a[0], |x, edge| if x < edge { 0.0 } else { 1.0 })
                }
                "smoothstep" => {
                    need(3)?;
                    let (e0, e1, x) = (a[0], a[1], a[2]);
                    let t = x.zip(e0, |x, e| x - e).zip(e1.zip(e0, |b, c| b - c), |n, d| if d == 0.0 { 0.0 } else { (n / d).clamp(0.0, 1.0) });
                    t.map(|t| t * t * (3.0 - 2.0 * t))
                }
                "length" => {
                    need(1)?;
                    Val::s((a[0].v[0].powi(2) + a[0].v[1].powi(2) + a[0].v[2].powi(2)).sqrt())
                }
                "dist" => {
                    need(2)?;
                    let d = a[0].zip(a[1], |x, y| x - y);
                    Val::s((d.v[0].powi(2) + d.v[1].powi(2) + d.v[2].powi(2)).sqrt())
                }
                "dot" => {
                    need(2)?;
                    Val::s(a[0].v[0] * a[1].v[0] + a[0].v[1] * a[1].v[1] + a[0].v[2] * a[1].v[2])
                }
                "noise" | "cnoise" => {
                    need(1)?;
                    let (x, y) = if a.len() >= 2 { (a[0].x(), a[1].x()) } else { (a[0].v[0], a[0].v[1]) };
                    let n = noise2(x, y);
                    if name == "cnoise" { Val::vec([noise2(x, y), noise2(x + 31.7, y + 17.3), noise2(x + 71.1, y + 43.9)]) } else { Val::s(n) }
                }
                "hash" | "rand" => {
                    need(1)?;
                    let (x, y) = if a.len() >= 2 { (a[0].x(), a[1].x()) } else { (a[0].v[0], a[0].v[1]) };
                    Val::s(hash01(x, y, 3.0))
                }
                "rgb" => {
                    need(3)?;
                    Val::vec([a[0].x(), a[1].x(), a[2].x()])
                }
                "hsv" => {
                    need(3)?;
                    Val::vec(hsv_to_rgb(a[0].x(), a[1].x().clamp(0.0, 1.0), a[2].x().clamp(0.0, 1.0)))
                }
                "gray" | "grey" => {
                    need(1)?;
                    let v = a[0];
                    Val::s(0.2126 * v.v[0] + 0.7152 * v.v[1] + 0.0722 * v.v[2])
                }
                other => return Err(format!("unknown function `{other}`")),
            }
        }
    })
}

impl Program {
    /// Evaluate at pixel `(x, y)` of a `w × h` image: straight RGB 0..1.
    pub fn eval_pixel(&self, x: u32, y: u32, w: u32, h: u32, time: f32) -> std::result::Result<[f32; 3], String> {
        let vars = [
            ("u".to_string(), Val::s((x as f32 + 0.5) / w.max(1) as f32)),
            ("v".to_string(), Val::s((y as f32 + 0.5) / h.max(1) as f32)),
            ("x".to_string(), Val::s(x as f32)),
            ("y".to_string(), Val::s(y as f32)),
            ("w".to_string(), Val::s(w as f32)),
            ("h".to_string(), Val::s(h as f32)),
            ("time".to_string(), Val::s(time)),
            ("PI".to_string(), Val::s(std::f32::consts::PI)),
        ];
        let mut env = Env { vars: &vars, locals: Vec::new() };
        for (n, e) in &self.assigns {
            let v = eval(e, &env)?;
            env.locals.push((n.clone(), v));
        }
        let r = eval(&self.result, &env)?;
        Ok([r.v[0].clamp(0.0, 1.0), r.v[1].clamp(0.0, 1.0), r.v[2].clamp(0.0, 1.0)])
    }
}

/// Built-in scripts, like the reference's starter set.
pub fn builtin() -> Vec<SeExprScript> {
    vec![
        SeExprScript { name: "Gradient U".into(), source: "[u, u, u]".into(), tags: vec!["generator".into()] },
        SeExprScript {
            name: "Radial".into(),
            source: "$d = dist([u, v, 0], [0.5, 0.5, 0]) * 2;\n[1 - $d, 1 - $d, 1 - $d]".into(),
            tags: vec!["generator".into()],
        },
        SeExprScript { name: "Checker".into(), source: "$c = (floor(u * 8) + floor(v * 8)) % 2;\n[$c, $c, $c]".into(), tags: vec!["generator".into()] },
        SeExprScript {
            name: "Clouds".into(),
            source: "$n = noise(u * 6, v * 6) * 0.6 + noise(u * 12, v * 12) * 0.3 + noise(u * 24, v * 24) * 0.1;\n[$n, $n, $n]".into(),
            tags: vec!["generator".into()],
        },
        SeExprScript { name: "Rainbow".into(), source: "hsv(u, 1, 1)".into(), tags: vec!["generator".into()] },
    ]
}

fn scripts(s: &mut Session) -> &mut Vec<SeExprScript> {
    if s.seexpr_scripts.is_empty() {
        s.seexpr_scripts = builtin();
    }
    &mut s.seexpr_scripts
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let items: Vec<Value> = scripts(s).iter().map(|x| json!({ "name": x.name, "source": x.source, "tags": x.tags })).collect();
    Ok(json!({ "scripts": items }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "scripts.seexpr.save";
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))?.to_string();
    let source = p.get("source").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `source`"))?.to_string();
    compile(&source).map_err(|e| bad(cmd, format!("script error: {e}")))?;
    let tags: Vec<String> =
        p.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let list = scripts(s);
    match list.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)) {
        Some(x) => {
            x.source = source;
            if !tags.is_empty() {
                x.tags = tags;
            }
        }
        None => {
            if list.len() >= MAX_SCRIPTS {
                return Err(bad(cmd, format!("at most {MAX_SCRIPTS} scripts")));
            }
            list.push(SeExprScript { name: name.clone(), source, tags });
        }
    }
    Ok(json!({ "name": name }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "scripts.seexpr.delete";
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `name`"))?;
    let list = scripts(s);
    let before = list.len();
    list.retain(|x| !x.name.eq_ignore_ascii_case(name));
    if list.len() == before {
        return Err(bad(cmd, format!("no script named `{name}`")));
    }
    Ok(json!({ "name": name }))
}

/// `scripts.seexpr.check`: compile a script and evaluate it at one pixel.
fn check(_: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "scripts.seexpr.check";
    let source = p.get("source").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `source`"))?;
    let prog = compile(source).map_err(|e| bad(cmd, format!("script error: {e}")))?;
    let sample = prog.eval_pixel(0, 0, 16, 16, 0.0).map_err(|e| bad(cmd, format!("script error: {e}")))?;
    Ok(json!({ "ok": true, "sample": sample }))
}

/// `fill.seexpr`: render a script (`name` or `source`) over the active layer (or the selection).
fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "fill.seexpr";
    let source = match (p.get("source").and_then(Value::as_str), p.get("name").and_then(Value::as_str)) {
        (Some(src), _) => src.to_string(),
        (None, Some(name)) => scripts(s)
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(name))
            .map(|x| x.source.clone())
            .ok_or_else(|| bad(cmd, format!("no script named `{name}`")))?,
        (None, None) => return Err(bad(cmd, "give `name` or `source`")),
    };
    let prog = compile(&source).map_err(|e| bad(cmd, format!("script error: {e}")))?;
    let time = p.get("time").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let opacity = p.get("opacity").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => redrob_photo_doc::LayerId(id),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))?,
    };
    let dmg = s.edit("Generator Script", |doc, _| {
        let (w, h) = (doc.size.width, doc.size.height);
        let area = doc.bounds();
        let sel = doc.selection.clone();
        let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = layer.surface_mut().ok_or_else(|| bad(cmd, "the layer has no pixels"))?;
        let fmt = surf.format();
        let nc = fmt.channels();
        let mut old = Vec::new();
        surf.read_region_into(area, &mut old);
        let mut data = vec![0.0f32; (w as usize) * (h as usize) * nc];
        let mut err: Option<String> = None;
        for y in 0..h {
            for x in 0..w {
                let c = match prog.eval_pixel(x, y, w, h, time) {
                    Ok(c) => c,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let i = (y * w + x) as usize;
                let k = opacity * sel.as_ref().map_or(1.0, |m| m.sample_channel(x as i32, y as i32, 0));
                let px = redrob_photo_raster::from_rgba(&fmt, [c[0], c[1], c[2], 1.0]);
                for ch in 0..nc {
                    let prev = old.get(i * nc + ch).copied().unwrap_or(0.0);
                    data[i * nc + ch] = prev + (px[ch] - prev) * k;
                }
            }
            if err.is_some() {
                break;
            }
        }
        if let Some(e) = err {
            return Err(bad(cmd, format!("script error: {e}")));
        }
        surf.write_region(area, &data);
        Ok(area)
    })?;
    if let Some(st) = s.active_mut() {
        st.last_damage = Some(dmg);
    }
    Ok(json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] }))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $en:expr, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, run: $run, journal: $journal }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("scripts.seexpr.list", "Generator Scripts", "{} → {scripts:[{name, source, tags}]}", always, list, false),
        spec!("scripts.seexpr.save", "Save Generator Script", r##"{"name":string,"source":string,"tags":[string]?}"##, always, save, true),
        spec!("scripts.seexpr.delete", "Delete Generator Script", r##"{"name":string}"##, always, delete, true),
        spec!("scripts.seexpr.check", "Check Generator Script", r##"{"source":string} → {ok, sample}"##, always, check, false),
        spec!(
            "fill.seexpr",
            "Fill with Generator Script",
            r##"{"name":string?|"source":string,"layer":id?=active,"opacity":0..1=1,"time":number=0}"##,
            has_paintable,
            fill,
            true
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assignments_vectors_and_functions() {
        let p = compile("$c = (floor(u * 8) + floor(v * 8)) % 2;\n[$c, 1 - $c, 0.5]").unwrap();
        let px = p.eval_pixel(0, 0, 16, 16, 0.0).unwrap();
        assert_eq!(px, [0.0, 1.0, 0.5]);
        let px = p.eval_pixel(2, 0, 16, 16, 0.0).unwrap();
        assert_eq!(px, [1.0, 0.0, 0.5]);
        assert!(compile("foo(1)").is_ok());
        assert!(compile("foo(1)").unwrap().eval_pixel(0, 0, 1, 1, 0.0).is_err());
        assert!(compile("1 +").is_err());
        let cond = compile("u < 0.5 ? [1,0,0] : hsv(0.5, 1, 1)").unwrap();
        assert_eq!(cond.eval_pixel(0, 0, 4, 4, 0.0).unwrap(), [1.0, 0.0, 0.0]);
    }
}
