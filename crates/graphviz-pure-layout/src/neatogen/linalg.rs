//! Small dense solvers: `lib/neatogen/solve.c` (Gaussian elimination),
//! `lu.c` (LU decomposition), `matinv.c` and `solveCircuit` (`circuit.c`).

/// `solve(a, b, c, n)`: solve `a b = c` for row-major `a[n][n]`. `a` and `c`
/// are restored; on an ill-conditioned matrix `b` is left unchanged.
pub fn solve(a: &mut [f64], b: &mut [f64], c: &mut [f64], n: usize) {
    let asave = a.to_vec();
    let csave = c.to_vec();
    let nm = n - 1;
    let restore = |a: &mut [f64], c: &mut [f64]| {
        a.copy_from_slice(&asave);
        c.copy_from_slice(&csave);
    };
    for i in 0..nm {
        let mut amax = 0.0;
        let mut istar = 0;
        for ii in i..n {
            let dum = a[ii * n + i].abs();
            if dum < amax {
                continue;
            }
            istar = ii;
            amax = dum;
        }
        if amax < 1.0e-10 {
            // C prints "ill-conditioned" and returns without restoring.
            return;
        }
        for j in i..n {
            a.swap(istar * n + j, i * n + j);
        }
        c.swap(istar, i);
        for ii in (i + 1)..n {
            let pivot = a[ii * n + i] / a[i * n + i];
            c[ii] = (-pivot).mul_add(c[i], c[ii]);
            for j in 0..n {
                a[ii * n + j] = (-pivot).mul_add(a[i * n + j], a[ii * n + j]);
            }
        }
    }
    if a[n * n - 1].abs() < 1.0e-10 {
        return;
    }
    b[n - 1] = c[n - 1] / a[n * n - 1];
    for k in 0..nm {
        let m = n - k - 2;
        b[m] = c[m];
        for j in (m + 1)..n {
            b[m] = (-a[m * n + j]).mul_add(b[j], b[m]);
        }
        b[m] /= a[m * n + m];
    }
    restore(a, c);
}

/// LU decomposition state (`lu_t`).
pub struct Lu {
    lu: Vec<Vec<f64>>,
    ps: Vec<usize>,
}

/// `lu_decompose(lu, a, n)`; `None` if `a` is singular.
pub fn lu_decompose(a: &[Vec<f64>], n: usize) -> Option<Lu> {
    let mut lu = vec![vec![0.0; n]; n];
    let mut ps = vec![0usize; n];
    let mut scales = vec![0.0; n];
    for i in 0..n {
        let mut biggest = 0.0f64;
        for j in 0..n {
            lu[i][j] = a[i][j];
            biggest = biggest.max(lu[i][j].abs());
        }
        if biggest > 0.0 {
            scales[i] = 1.0 / biggest;
        } else {
            return None;
        }
        ps[i] = i;
    }
    let mut pivotindex = 0;
    for k in 0..n.saturating_sub(1) {
        let mut biggest = 0.0;
        for i in k..n {
            let tempf = lu[ps[i]][k].abs() * scales[ps[i]];
            if biggest < tempf {
                biggest = tempf;
                pivotindex = i;
            }
        }
        if biggest <= 0.0 {
            return None;
        }
        if pivotindex != k {
            ps.swap(k, pivotindex);
        }
        let pivot = lu[ps[k]][k];
        for i in (k + 1)..n {
            let mult = lu[ps[i]][k] / pivot;
            lu[ps[i]][k] = mult;
            for j in (k + 1)..n {
                let v = lu[ps[k]][j];
                lu[ps[i]][j] = (-mult).mul_add(v, lu[ps[i]][j]);
            }
        }
    }
    if lu[ps[n - 1]][n - 1] == 0.0 {
        return None;
    }
    Some(Lu { lu, ps })
}

/// `lu_solve(lu, x, bi, n)`: solve with the unit vector `e_bi`.
pub fn lu_solve(lu: &Lu, x: &mut [f64], bi: usize, n: usize) {
    for i in 0..n {
        let mut dot = 0.0;
        for j in 0..i {
            dot = lu.lu[lu.ps[i]][j].mul_add(x[j], dot);
        }
        x[i] = f64::from(u8::from(lu.ps[i] == bi)) - dot;
    }
    for i in (0..n).rev() {
        let mut dot = 0.0;
        for j in (i + 1)..n {
            dot = lu.lu[lu.ps[i]][j].mul_add(x[j], dot);
        }
        x[i] = (x[i] - dot) / lu.lu[lu.ps[i]][i];
    }
}

/// `matinv(A, Ainv, n)`; false if singular.
pub fn matinv(a: &[Vec<f64>], ainv: &mut [Vec<f64>], n: usize) -> bool {
    let Some(lu) = lu_decompose(a, n) else {
        return false;
    };
    for i in 0..n {
        lu_solve(&lu, &mut ainv[i], i, n);
    }
    for i in 0..n {
        for j in 0..i {
            let t = ainv[i][j];
            ainv[i][j] = ainv[j][i];
            ainv[j][i] = t;
        }
    }
    true
}

/// `solveCircuit(nG, Gm, Gm_inv)`.
pub fn solve_circuit(ng: usize, gm: &mut [Vec<f64>], gm_inv: &mut [Vec<f64>]) -> bool {
    graphviz_pure_core::verbose_msg!("Calculating circuit model");
    for i in 0..ng {
        let mut sum = 0.0;
        for j in 0..ng {
            if i != j {
                sum += gm[i][j];
            }
        }
        gm[i][i] = -sum;
    }
    matinv(gm, gm_inv, ng - 1)
}
