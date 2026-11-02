// SPDX-License-Identifier: GPL-3.0-or-later
// Port of Stronghold Protocol 41bbca57 server/sim/detmath.js (fdlibm algorithms).
// Keep expression order; no FMA, fast-math or host trigonometric intrinsics.
#![allow(clippy::excessive_precision, clippy::approx_constant)]
fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}
fn lo(x: f64) -> u32 {
    x.to_bits() as u32
}
fn words(h: i32, l: u32) -> f64 {
    f64::from_bits(((h as u32 as u64) << 32) | l as u64)
}
fn pow2(e: i32) -> f64 {
    words((e + 1023) << 20, 0)
}

pub fn hypot(x: f64, y: f64) -> f64 {
    if x.is_infinite() || y.is_infinite() {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let m = x.abs().max(y.abs());
    if m > pow2(500) {
        let a = x * pow2(-600);
        let b = y * pow2(-600);
        return (a * a + b * b).sqrt() * pow2(600);
    }
    if m < pow2(-500) && m != 0.0 {
        let a = x * pow2(600);
        let b = y * pow2(600);
        return (a * a + b * b).sqrt() * pow2(-600);
    }
    (x * x + y * y).sqrt()
}
pub fn round_half_even(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = x.floor();
    let result = if x - f == 0.5 {
        if f % 2.0 == 0.0 {
            f
        } else {
            f + 1.0
        }
    } else if x - f < 0.5 {
        f
    } else {
        f + 1.0
    };
    result + 0.0
}
fn mul2(ah: f64, al: f64, bh: f64, bl: f64) -> (f64, f64) {
    let p = ah * bh;
    let t = 134217729.0 * ah;
    let ahh = t - (t - ah);
    let ahl = ah - ahh;
    let t = 134217729.0 * bh;
    let bhh = t - (t - bh);
    let bhl = bh - bhh;
    let e = ((ahh * bhh - p) + ahh * bhl + ahl * bhh) + ahl * bhl + (ah * bl + al * bh);
    let s = p + e;
    (s, e - (s - p))
}
/// Restricted to the finite integer-exponent domain used by the simulator.
/// Outside this domain, fail instead of using platform-dependent powf.
pub fn powi(x: f64, n: i32) -> Result<f64, &'static str> {
    if !x.is_finite() || x.abs() > pow2(500) {
        return Err("powi base outside deterministic domain");
    }
    if x == 0.0 {
        let odd = n % 2 != 0;
        return Ok(if n == 0 {
            1.0
        } else if n > 0 {
            if odd {
                x
            } else {
                0.0
            }
        } else if odd {
            1.0 / x
        } else {
            f64::INFINITY
        });
    }
    let mut k = n.unsigned_abs();
    let (mut a, mut b) = ((1.0, 0.0), (x, 0.0));
    while k > 0 {
        if k % 2 == 1 {
            a = mul2(a.0, a.1, b.0, b.1);
        }
        k /= 2;
        if k > 0 {
            b = mul2(b.0, b.1, b.0, b.1);
        }
    }
    let result = a.0 + a.1;
    Ok(if n < 0 { 1.0 / result } else { result })
}
const S: [f64; 6] = [
    -1.66666666666666324348e-01,
    8.33333333332248946124e-03,
    -1.98412698298579493134e-04,
    2.75573137070700676789e-06,
    -2.50507602534068634195e-08,
    1.58969099521155010221e-10,
];
const C: [f64; 6] = [
    4.16666666666666019037e-02,
    -1.38888888888741095749e-03,
    2.48015872894767294178e-05,
    -2.75573143513906633035e-07,
    2.08757232129817482790e-09,
    -1.13596475577881948265e-11,
];
fn ksin(x: f64, y: f64, tail: bool) -> f64 {
    if (hi(x) & 0x7fffffff) < 0x3e400000 && x.trunc() == 0.0 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = S[1] + z * (S[2] + z * (S[3] + z * (S[4] + z * S[5])));
    if !tail {
        x + v * (S[0] + z * r)
    } else {
        x - ((z * (0.5 * y - v * r) - y) - v * S[0])
    }
}
fn kcos(x: f64, y: f64) -> f64 {
    let ix = hi(x) & 0x7fffffff;
    if ix < 0x3e400000 && x.trunc() == 0.0 {
        return 1.0;
    }
    let z = x * x;
    let r = z * (C[0] + z * (C[1] + z * (C[2] + z * (C[3] + z * (C[4] + z * C[5])))));
    if ix < 0x3fd33333 {
        return 1.0 - (0.5 * z - (z * r - x * y));
    }
    let qx = if ix > 0x3fe90000 {
        0.28125
    } else {
        words(ix - 0x00200000, 0)
    };
    (1.0 - qx) - ((0.5 * z - qx) - (z * r - x * y))
}
const NPIO2: [i32; 32] = [
    0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C, 0x4025FDBB, 0x402921FB,
    0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C, 0x40346B9C, 0x4035FDBB, 0x40378FDB, 0x403921FB,
    0x403AB41B, 0x403C463A, 0x403DD85A, 0x403F6A7A, 0x40407E4C, 0x4041475C, 0x4042106C, 0x4042D97C,
    0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB, 0x4046C6CB, 0x40478FDB, 0x404858EB, 0x404921FB,
];
const P1: f64 = 1.57079632673412561417;
const P1T: f64 = 6.07710050650619224932e-11;
const P2: f64 = 6.07710050630396597660e-11;
const P2T: f64 = 2.02226624879595063154e-21;
const P3: f64 = 2.02226624871116645580e-21;
const P3T: f64 = 8.47842766036889956997e-32;
fn rem_pio2(x0: f64) -> (i32, f64, f64) {
    let x = if (hi(x0) & 0x7fffffff) > 0x413921fb {
        x0 % 6.283185307179586
    } else {
        x0
    };
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return (0, x, 0.0);
    }
    if ix < 0x4002d97c {
        if hx > 0 {
            let mut z = x - P1;
            if ix != 0x3ff921fb {
                let y = z - P1T;
                return (1, y, (z - y) - P1T);
            }
            z -= P2;
            let y = z - P2T;
            return (1, y, (z - y) - P2T);
        }
        let mut z = x + P1;
        if ix != 0x3ff921fb {
            let y = z + P1T;
            return (-1, y, (z - y) + P1T);
        }
        z += P2;
        let y = z + P2T;
        return (-1, y, (z - y) + P2T);
    }
    let mut t = x.abs();
    let n = (t * 6.36619772367581382433e-01 + 0.5).trunc() as i32;
    let mut r = t - n as f64 * P1;
    let mut w = n as f64 * P1T;
    let mut y = r - w;
    if !(n < 32 && ix != NPIO2[(n - 1) as usize]) {
        let j = ix >> 20;
        if j - ((hi(y) >> 20) & 0x7ff) > 16 {
            t = r;
            w = n as f64 * P2;
            r = t - w;
            w = n as f64 * P2T - ((t - r) - w);
            y = r - w;
            if j - ((hi(y) >> 20) & 0x7ff) > 49 {
                t = r;
                w = n as f64 * P3;
                r = t - w;
                w = n as f64 * P3T - ((t - r) - w);
                y = r - w;
            }
        }
    }
    let tail = (r - y) - w;
    if hx < 0 {
        (-n, -y, -tail)
    } else {
        (n, y, tail)
    }
}
pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if (hi(x) & 0x7fffffff) <= 0x3fe921fb {
        return ksin(x, 0.0, false);
    }
    let (n, y, t) = rem_pio2(x);
    match n & 3 {
        0 => ksin(y, t, true),
        1 => kcos(y, t),
        2 => -ksin(y, t, true),
        _ => -kcos(y, t),
    }
}
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if (hi(x) & 0x7fffffff) <= 0x3fe921fb {
        return kcos(x, 0.0);
    }
    let (n, y, t) = rem_pio2(x);
    match n & 3 {
        0 => kcos(y, t),
        1 => -ksin(y, t, true),
        2 => -kcos(y, t),
        _ => ksin(y, t, true),
    }
}
const AH: [f64; 4] = [
    4.63647609000806093515e-01,
    7.85398163397448278999e-01,
    9.82793723247329054082e-01,
    1.57079632679489655800,
];
const AL: [f64; 4] = [
    2.26987774529616870924e-17,
    3.06161699786838301793e-17,
    1.39033110312309984516e-17,
    6.12323399573676603587e-17,
];
const AT: [f64; 11] = [
    3.33333333333329318027e-01,
    -1.99999999998764832476e-01,
    1.42857142725034663711e-01,
    -1.11111104054623557880e-01,
    9.09088713343650656196e-02,
    -7.69187620504482999495e-02,
    6.66107313738753120669e-02,
    -5.83357013379057348645e-02,
    4.97687799461593236017e-02,
    -3.65315727442169155270e-02,
    1.62858201153657823623e-02,
];
fn atan(x0: f64) -> f64 {
    let mut x = x0;
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    let id: i32;
    if ix >= 0x44100000 {
        if ix > 0x7ff00000 || (ix == 0x7ff00000 && lo(x) != 0) {
            return x + x;
        }
        return if hx > 0 {
            AH[3] + AL[3]
        } else {
            -AH[3] - AL[3]
        };
    }
    if ix < 0x3fdc0000 {
        if ix < 0x3e200000 {
            return x;
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3ff30000 {
            if ix < 0x3fe60000 {
                id = 0;
                x = (2.0 * x - 1.0) / (2.0 + x);
            } else {
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x40038000 {
            id = 2;
            x = (x - 1.5) / (1.0 + 1.5 * x);
        } else {
            id = 3;
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id < 0 {
        return x - x * (s1 + s2);
    }
    let r = AH[id as usize] - ((x * (s1 + s2) - AL[id as usize]) - x);
    if hx < 0 {
        -r
    } else {
        r
    }
}
const PI: f64 = 3.1415926535897931160;
const PI2: f64 = 1.5707963267948965580;
const PI4: f64 = 7.8539816339744827900e-1;
const PIL: f64 = 1.2246467991473532e-16;
pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    let lx = lo(x);
    let hy = hi(y);
    let iy = hy & 0x7fffffff;
    let ly = lo(y);
    if x == 1.0 {
        return atan(y);
    }
    let mut m = ((hy as u32 >> 31) & 1) | ((hx as u32 >> 30) & 2);
    if (iy as u32 | ly) == 0 {
        return match m {
            2 => PI,
            3 => -PI,
            _ => y,
        };
    }
    if (ix as u32 | lx) == 0 {
        return if hy < 0 { -PI2 } else { PI2 };
    }
    if ix == 0x7ff00000 {
        if iy == 0x7ff00000 {
            return [PI4, -PI4, 3.0 * PI4, -3.0 * PI4][m as usize];
        }
        return [0.0, -0.0, PI, -PI][m as usize];
    }
    if iy == 0x7ff00000 {
        return if hy < 0 { -PI2 } else { PI2 };
    }
    let k = (iy - ix) >> 20;
    let z = if k > 60 {
        m &= 1;
        PI2 + 0.5 * PIL
    } else if hx < 0 && k < -60 {
        0.0
    } else {
        atan((y / x).abs())
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PIL),
        _ => (z - PIL) - PI,
    }
}
