//! Ruby's `format("%.Nf", x)`. Ruby formats floats with its own copy of David Gay's `dtoa`
//! (missing/dtoa.c, mode 3), which carries one local change: in the floating-point quick path, a
//! value within epsilon of a rounding tie is rounded to even on the last digit generated, without
//! checking which side of the tie it is on (2.675 → "2.68", 9.995 → "10.00"). Everything else
//! rounds exactly, so this ports that quick path and leaves the rest to Rust's exact formatting.

const TENS: [f64; 23] =
    [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22];
const BIGTENS: [f64; 5] = [1e16, 1e32, 1e64, 1e128, 1e256];
const TEN_PMAX: i32 = 22;
const QUICK_MAX: i32 = 14;
const BLETCH: i32 = 0x10;

/// What the quick path decided: the digits and the position of the decimal point (dtoa's `decpt`).
struct Digits {
    digits: Vec<u8>,
    decpt: i32,
}

enum Quick {
    Decided(Digits),
    /// Within epsilon of a tie with an even last digit: the exact path runs with its `half` flag
    /// set, which suppresses a round-up there unless the digit is odd.
    Half,
    /// Nothing decided: the exact path rounds half to even.
    Exact,
}

/// `s = bump_up`: increment the last digit, dropping trailing 9s; all 9s become "1" a place higher.
fn bump_up(digits: &mut Vec<u8>, k: &mut i32) {
    loop {
        match digits.pop() {
            Some(b'9') if digits.is_empty() => {
                *k += 1;
                digits.push(b'1');
                return;
            }
            Some(b'9') => continue,
            Some(d) => {
                digits.push(d + 1);
                return;
            }
            None => return,
        }
    }
}

/// dtoa mode 3 (`ndigits` places after the decimal point), floating-point quick path only.
/// None means "fall back to the exact algorithm".
fn quick(x: f64, ndigits: i32) -> Quick {
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    if biased == 0 {
        return Quick::Exact; // denormal
    }
    let d2 = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let i = biased - 1023;
    let ds = (d2 - 1.5) * 0.289529654602168 + 0.1760912590558 + i as f64 * 0.301029995663981;
    let mut k = ds as i32;
    if ds < 0.0 && ds != k as f64 {
        k -= 1;
    }
    let mut k_check = true;
    if (0..=TEN_PMAX).contains(&k) {
        if x < TENS[k as usize] {
            k -= 1;
        }
        k_check = false;
    }
    let mut ilim = ndigits + k + 1;
    let ilim1 = ilim - 1;
    if !(ilim >= 0 && ilim <= QUICK_MAX) {
        return Quick::Exact;
    }

    let mut d = x;
    let mut ieps: i32 = 2;
    let mut idx = 0usize;
    if k > 0 {
        let mut ds = TENS[(k & 0xf) as usize];
        let mut j = k >> 4;
        if j & BLETCH != 0 {
            j &= BLETCH - 1;
            d /= BIGTENS[4];
            ieps += 1;
        }
        while j != 0 {
            if j & 1 != 0 {
                ieps += 1;
                ds *= BIGTENS[idx];
            }
            j >>= 1;
            idx += 1;
        }
        d /= ds;
    } else if k < 0 {
        let j1 = -k;
        d *= TENS[(j1 & 0xf) as usize];
        let mut j = j1 >> 4;
        while j != 0 {
            if j & 1 != 0 {
                ieps += 1;
                d *= BIGTENS[idx];
            }
            j >>= 1;
            idx += 1;
        }
    }
    if k_check && d < 1.0 && ilim > 0 {
        if ilim1 <= 0 {
            return Quick::Exact;
        }
        ilim = ilim1;
        k -= 1;
        d *= 10.0;
        ieps += 1;
    }
    let mut eps = (ieps as f64 * d + 7.0) * 2f64.powi(-52);
    if ilim == 0 {
        d -= 5.0;
        if d > eps {
            return Quick::Decided(Digits { digits: b"1".to_vec(), decpt: k + 2 }); // one_digit
        }
        if d < -eps {
            return Quick::Decided(Digits { digits: vec![], decpt: -ndigits }); // no_digits
        }
        return Quick::Exact;
    }
    eps *= TENS[(ilim - 1) as usize];
    let mut digits: Vec<u8> = vec![];
    let mut i = 1;
    loop {
        let l = d as i64;
        d -= l as f64;
        if d == 0.0 {
            ilim = i;
        }
        digits.push(b'0' + l as u8);
        if i == ilim {
            if d > 0.5 + eps {
                bump_up(&mut digits, &mut k);
                return Quick::Decided(Digits { digits, decpt: k + 1 });
            } else if d < 0.5 - eps {
                while digits.last() == Some(&b'0') {
                    digits.pop();
                }
                return Quick::Decided(Digits { digits, decpt: k + 1 });
            }
            // within epsilon of a tie: Ruby's change rounds to even on the digit just generated
            if (digits[digits.len() - 1] - b'0') & 1 == 1 {
                bump_up(&mut digits, &mut k);
                return Quick::Decided(Digits { digits, decpt: k + 1 });
            }
            return Quick::Half;
        }
        i += 1;
        d *= 10.0;
    }
}

/// The exact path with `half` set: the exact truncation, rounded up only when its last digit is
/// odd and the discarded part is at least half.
fn exact_with_half(x: f64, prec: usize) -> String {
    let long = format!("{x:.*}", prec + 60);
    let (int_part, frac) = long.split_once('.').expect("a fraction");
    let mut kept: Vec<u8> = int_part.bytes().chain(frac.bytes().take(prec)).collect();
    let rest = &frac.as_bytes()[prec..];
    let at_least_half = rest[0] >= b'5';
    let odd = (kept[kept.len() - 1] - b'0') & 1 == 1;
    if odd && at_least_half {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let int_len = kept.len() - prec;
    let (i, f) = kept.split_at(int_len);
    let (i, f) = (String::from_utf8_lossy(i), String::from_utf8_lossy(f));
    if prec == 0 { i.into_owned() } else { format!("{i}.{f}") }
}

/// `format("%.{prec}f", x)`.
pub fn format_f(x: f64, prec: usize) -> String {
    if !x.is_finite() || x <= 0.0 {
        return format!("{x:.prec$}");
    }
    let Digits { digits, decpt } = match quick(x, prec as i32) {
        Quick::Decided(d) => d,
        Quick::Half => return exact_with_half(x, prec),
        Quick::Exact => return format!("{x:.prec$}"),
    };
    let text: String = digits.iter().map(|d| *d as char).collect();
    let (int_part, frac_part) = if decpt <= 0 {
        ("0".to_string(), format!("{}{}", "0".repeat((-decpt) as usize), text))
    } else if text.len() as i32 >= decpt {
        (text[..decpt as usize].to_string(), text[decpt as usize..].to_string())
    } else {
        (format!("{text}{}", "0".repeat(decpt as usize - text.len())), String::new())
    };
    let mut frac = frac_part;
    while frac.len() < prec {
        frac.push('0');
    }
    if prec == 0 { int_part } else { format!("{int_part}.{}", &frac[..prec]) }
}

#[cfg(test)]
mod tests {
    use super::format_f;

    #[test]
    fn ruby_rounds_near_ties_to_even_on_the_last_digit_generated() {
        assert_eq!(format_f(2.675, 2), "2.68");
        assert_eq!(format_f(9.995, 2), "10.00");
        assert_eq!(format_f(1.115, 2), "1.12");
        // an even last digit falls back to the exact answer
        assert_eq!(format_f(1.005, 2), "1.00");
        assert_eq!(format_f(8.345, 2), "8.34");
        assert_eq!(format_f(0.285, 2), "0.28");
        assert_eq!(format_f(0.125, 2), "0.12");
        assert_eq!(format_f(0.375, 2), "0.38");
    }

    #[test]
    fn ordinary_values_and_the_edges() {
        assert_eq!(format_f(0.0, 2), "0.00");
        assert_eq!(format_f(14.0, 0), "14");
        assert_eq!(format_f(2.5, 0), "2");
        assert_eq!(format_f(3.5, 0), "4");
        assert_eq!(format_f(1234.5678, 1), "1234.6");
        assert_eq!(format_f(0.004, 2), "0.00");
        assert_eq!(format_f(0.006, 2), "0.01");
    }
}
