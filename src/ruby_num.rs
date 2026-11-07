//! Ruby's Integer/Float arithmetic where it shows in costs: an Integer times a Float is a Float, an
//! Integer sum stays exact, and `sum` switches to Kahan-Babuska compensation at the first Float.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i128),
    Float(f64),
}

impl Num {
    /// A literal as Ruby would read it: `10` is an Integer, `12.5` a Float.
    pub fn literal(text: &str) -> Num {
        match text.parse::<i128>() {
            Ok(i) => Num::Int(i),
            Err(_) => Num::Float(text.parse().expect("a number literal")),
        }
    }

    pub fn from_yaml(n: &serde_yaml::Number) -> Num {
        match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => Num::Int(i as i128),
            (None, Some(u)) => Num::Int(u as i128),
            _ => Num::Float(n.as_f64().unwrap_or(0.0)),
        }
    }

    pub fn to_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }

    pub fn mul(self, other: Num) -> Num {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Num::Int(a * b),
            (a, b) => Num::Float(a.to_f64() * b.to_f64()),
        }
    }

    pub fn json(self) -> Value {
        match self {
            Num::Int(i) => Value::from(i as i64),
            Num::Float(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
        }
    }
}

/// `terms.sum`: exact while every term is an Integer, then Kahan-Babuska over Floats.
pub fn sum(terms: impl IntoIterator<Item = Num>) -> Num {
    let mut ints: i128 = 0;
    let mut it = terms.into_iter();
    let mut first_float = None;
    for t in it.by_ref() {
        match t {
            Num::Int(i) => ints += i,
            Num::Float(f) => {
                first_float = Some(f);
                break;
            }
        }
    }
    let Some(first) = first_float else { return Num::Int(ints) };
    let (mut f, mut c) = (ints as f64, 0.0_f64);
    let mut add = |x: f64| {
        if f.is_nan() {
            return;
        }
        if x.is_nan() {
            f = x;
            return;
        }
        if x.is_infinite() {
            f = if f.is_infinite() && x.is_sign_negative() != f.is_sign_negative() { f64::NAN } else { x };
            return;
        }
        if f.is_infinite() {
            return;
        }
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    };
    add(first);
    for t in it {
        add(t.to_f64());
    }
    Num::Float(f + c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_stay_exact_and_floats_compensate() {
        assert_eq!(sum([Num::Int(2), Num::Int(3)]), Num::Int(5));
        assert_eq!(sum([]), Num::Int(0));
        // 0.1 added ten times is 0.9999999999999999 naively, 1.0 compensated
        assert_eq!(sum(std::iter::repeat_n(Num::Float(0.1), 10)), Num::Float(1.0));
        assert_eq!(sum([Num::Int(1), Num::Float(0.5), Num::Int(2)]), Num::Float(3.5));
    }

    #[test]
    fn products_follow_the_operand_types() {
        assert_eq!(Num::Int(3).mul(Num::Int(4)), Num::Int(12));
        assert_eq!(Num::Int(3).mul(Num::Float(0.5)), Num::Float(1.5));
        assert_eq!(Num::literal("12.5"), Num::Float(12.5));
        assert_eq!(Num::literal("10"), Num::Int(10));
    }
}
