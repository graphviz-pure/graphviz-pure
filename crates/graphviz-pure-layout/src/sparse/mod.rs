//! Port of the parts of Graphviz `lib/sparse` (`SparseMatrix.c`,
//! `general.c`) used by overlap removal and the sfdp/neato smoothers.
//!
//! Matrices are real-valued or pattern-only, in CSR or coordinate format.
//! Entry order within a row follows C exactly (it determines summation
//! order in the solvers).

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use graphviz_pure_core::math;

pub mod matrix_ext;
pub mod quad_tree;

/// `MACHINEACC` (`general.h`).
pub const MACHINEACC: f64 = 1.0e-16;
/// `SYMMETRY_EPSILON` (`SparseMatrix.h`).
pub const SYMMETRY_EPSILON: f64 = 0.000_000_1;
/// `MINDIST` (`general.h`).
pub const MINDIST: f64 = 1.0e-15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csr,
    Coord,
}

/// `SparseMatrix_struct`. `a` is `None` for `MATRIX_TYPE_PATTERN`.
#[derive(Debug, Clone)]
pub struct SparseMatrix {
    pub m: usize,
    pub n: usize,
    pub nz: usize,
    pub ia: Vec<i32>,
    pub ja: Vec<i32>,
    pub a: Option<Vec<f64>>,
    pub format: Format,
    pub is_symmetric: bool,
    pub is_pattern_symmetric: bool,
}

impl SparseMatrix {
    /// `SparseMatrix_new(m, n, nz, MATRIX_TYPE_REAL|PATTERN, format)`.
    pub fn new(m: usize, n: usize, real: bool, format: Format) -> Self {
        SparseMatrix {
            m,
            n,
            nz: 0,
            ia: if format == Format::Csr {
                vec![0; m + 1]
            } else {
                Vec::new()
            },
            ja: Vec::new(),
            a: real.then(Vec::new),
            format,
            is_symmetric: false,
            is_pattern_symmetric: false,
        }
    }

    pub fn is_real(&self) -> bool {
        self.a.is_some()
    }

    /// `SparseMatrix_coordinate_form_add_entry(A, irn, jcn, val)`.
    pub fn coordinate_form_add_entry(&mut self, irn: i32, jcn: i32, val: f64) {
        debug_assert_eq!(self.format, Format::Coord);
        self.ia.push(irn);
        self.ja.push(jcn);
        if let Some(a) = self.a.as_mut() {
            a.push(val);
        }
        if irn as usize >= self.m {
            self.m = irn as usize + 1;
        }
        if jcn as usize >= self.n {
            self.n = jcn as usize + 1;
        }
        self.nz += 1;
    }

    /// `SparseMatrix_from_coordinate_format(A)`: CSR with repeated entries summed.
    pub fn from_coordinate_format(&self) -> SparseMatrix {
        from_coordinate_arrays(
            self.nz,
            self.m,
            self.n,
            &self.ia,
            &self.ja,
            self.a.as_deref(),
            true,
        )
    }

    /// `SparseMatrix_transpose(A)`.
    pub fn transpose(&self) -> SparseMatrix {
        let (m, n, nz) = (self.m, self.n, self.nz);
        let mut ib = vec![0i32; n + 1];
        let mut jb = vec![0i32; nz];
        let mut b = self.a.as_ref().map(|_| vec![0.0f64; nz]);
        for i in 0..m {
            for j in self.ia[i]..self.ia[i + 1] {
                ib[self.ja[j as usize] as usize + 1] += 1;
            }
        }
        for i in 0..n {
            ib[i + 1] += ib[i];
        }
        for i in 0..m {
            for j in self.ia[i]..self.ia[i + 1] {
                let c = self.ja[j as usize] as usize;
                let pos = ib[c] as usize;
                jb[pos] = i as i32;
                if let (Some(b), Some(a)) = (b.as_mut(), self.a.as_ref()) {
                    b[pos] = a[j as usize];
                }
                ib[c] += 1;
            }
        }
        for i in (0..n).rev() {
            ib[i + 1] = ib[i];
        }
        ib[0] = 0;
        SparseMatrix {
            m: n,
            n: m,
            nz,
            ia: ib,
            ja: jb,
            a: b,
            format: Format::Csr,
            is_symmetric: false,
            is_pattern_symmetric: false,
        }
    }

    /// `SparseMatrix_is_symmetric(A, test_pattern_symmetry_only)`.
    pub fn is_symmetric(&mut self, pattern_only: bool) -> bool {
        if self.is_symmetric {
            return true;
        }
        if pattern_only && self.is_pattern_symmetric {
            return true;
        }
        if self.m != self.n {
            return false;
        }
        let b = self.transpose();
        let m = self.m;
        let mut mask = vec![-1i32; m];
        let real = !pattern_only && self.is_real();
        if real {
            for i in 0..=m {
                if self.ia[i] != b.ia[i] {
                    return false;
                }
            }
        }
        for i in 0..m {
            for j in self.ia[i]..self.ia[i + 1] {
                mask[self.ja[j as usize] as usize] = j;
            }
            for j in b.ia[i]..b.ia[i + 1] {
                if mask[b.ja[j as usize] as usize] < self.ia[i] {
                    return false;
                }
            }
            if real {
                let (a, bb) = (self.a.as_ref().unwrap(), b.a.as_ref().unwrap());
                for j in b.ia[i]..b.ia[i + 1] {
                    let k = mask[b.ja[j as usize] as usize] as usize;
                    if (bb[j as usize] - a[k]).abs() > SYMMETRY_EPSILON {
                        return false;
                    }
                }
            }
        }
        if !pattern_only {
            self.is_symmetric = true;
        }
        self.is_pattern_symmetric = true;
        true
    }

    /// `SparseMatrix_symmetrize(A, pattern_symmetric_only)`.
    pub fn symmetrize(&mut self, pattern_only: bool) -> SparseMatrix {
        if self.is_symmetric(pattern_only) {
            return self.clone();
        }
        let b = self.transpose();
        let mut c = self.add(&b);
        c.is_symmetric = true;
        c.is_pattern_symmetric = true;
        c
    }

    /// `SparseMatrix_add(A, B)`.
    pub fn add(&self, b: &SparseMatrix) -> SparseMatrix {
        let (m, n) = (self.m, self.n);
        let mut ic = vec![0i32; m + 1];
        let mut jc = Vec::with_capacity(self.nz + b.nz);
        let real = self.is_real();
        let mut c: Vec<f64> = Vec::with_capacity(if real { self.nz + b.nz } else { 0 });
        let mut mask = vec![-1i32; n];
        let mut nz = 0i32;
        for i in 0..m {
            for j in self.ia[i]..self.ia[i + 1] {
                let col = self.ja[j as usize];
                mask[col as usize] = nz;
                jc.push(col);
                if real {
                    c.push(self.a.as_ref().unwrap()[j as usize]);
                }
                nz += 1;
            }
            for j in b.ia[i]..b.ia[i + 1] {
                let col = b.ja[j as usize];
                if mask[col as usize] < ic[i] {
                    jc.push(col);
                    if real {
                        c.push(b.a.as_ref().unwrap()[j as usize]);
                    }
                    nz += 1;
                } else if real {
                    c[mask[col as usize] as usize] += b.a.as_ref().unwrap()[j as usize];
                }
            }
            ic[i + 1] = nz;
        }
        SparseMatrix {
            m,
            n,
            nz: nz as usize,
            ia: ic,
            ja: jc,
            a: real.then_some(c),
            format: Format::Csr,
            is_symmetric: false,
            is_pattern_symmetric: false,
        }
    }

    /// `SparseMatrix_remove_diagonal(A)` (in place).
    pub fn remove_diagonal(&mut self) {
        let mut nz = 0usize;
        let mut sta = self.ia[0];
        for i in 0..self.m {
            for j in sta..self.ia[i + 1] {
                let j = j as usize;
                if self.ja[j] != i as i32 {
                    self.ja[nz] = self.ja[j];
                    if let Some(a) = self.a.as_mut() {
                        a[nz] = a[j];
                    }
                    nz += 1;
                }
            }
            sta = self.ia[i + 1];
            self.ia[i + 1] = nz as i32;
        }
        self.nz = nz;
        self.ja.truncate(nz);
        if let Some(a) = self.a.as_mut() {
            a.truncate(nz);
        }
    }

    /// `SparseMatrix_get_real_adjacency_matrix_symmetrized(A)`.
    pub fn real_adjacency_symmetrized(&self) -> SparseMatrix {
        let mut b = SparseMatrix {
            a: None,
            ..self.clone()
        };
        b.is_symmetric = false;
        b.is_pattern_symmetric = false;
        let mut s = b.symmetrize(true);
        s.remove_diagonal();
        s.a = Some(vec![1.0; s.nz]);
        s
    }

    /// `SparseMatrix_multiply_dense(A, v, res, dim)`.
    pub fn multiply_dense(&self, v: &[f64], res: &mut [f64], dim: usize) {
        let a = self.a.as_ref().unwrap();
        for i in 0..self.m {
            for k in 0..dim {
                res[i * dim + k] = 0.0;
            }
            for j in self.ia[i]..self.ia[i + 1] {
                let (j, c) = (j as usize, self.ja[j as usize] as usize);
                for k in 0..dim {
                    res[i * dim + k] = math::fma(a[j], v[c * dim + k], res[i * dim + k]);
                }
            }
        }
    }

    /// `SparseMatrix_multiply_vector(A, v, &res)`.
    pub fn multiply_vector(&self, v: &[f64], u: &mut [f64]) {
        let a = self.a.as_ref().unwrap();
        for i in 0..self.m {
            u[i] = 0.0;
            for j in self.ia[i]..self.ia[i + 1] {
                u[i] = math::fma(a[j as usize], v[self.ja[j as usize] as usize], u[i]);
            }
        }
    }
}

/// `SparseMatrix_from_coordinate_arrays(nz, m, n, irn, jcn, val, ...)`.
pub fn from_coordinate_arrays(
    nz: usize,
    m: usize,
    n: usize,
    irn: &[i32],
    jcn: &[i32],
    val: Option<&[f64]>,
    sum_repeated: bool,
) -> SparseMatrix {
    let mut ia = vec![0i32; m + 1];
    let mut ja = vec![0i32; nz];
    let mut a = val.map(|_| vec![0.0f64; nz]);
    for &r in irn.iter().take(nz) {
        ia[r as usize + 1] += 1;
    }
    for i in 0..m {
        ia[i + 1] += ia[i];
    }
    for i in 0..nz {
        let r = irn[i] as usize;
        let pos = ia[r] as usize;
        if let (Some(a), Some(v)) = (a.as_mut(), val) {
            a[pos] = v[i];
        }
        ja[pos] = jcn[i];
        ia[r] += 1;
    }
    for i in (1..=m).rev() {
        ia[i] = ia[i - 1];
    }
    ia[0] = 0;
    let mut s = SparseMatrix {
        m,
        n,
        nz,
        ia,
        ja,
        a,
        format: Format::Csr,
        is_symmetric: false,
        is_pattern_symmetric: false,
    };
    if sum_repeated {
        s.sum_repeat_entries();
    }
    s
}

impl SparseMatrix {
    /// `SparseMatrix_sum_repeat_entries(A)` (in place).
    pub fn sum_repeat_entries(&mut self) {
        let mut mask = vec![-1i32; self.n];
        let mut nz = 0usize;
        let mut sta = self.ia[0];
        for i in 0..self.m {
            for j in sta..self.ia[i + 1] {
                let j = j as usize;
                let c = self.ja[j] as usize;
                if mask[c] < self.ia[i] {
                    self.ja[nz] = self.ja[j];
                    if let Some(a) = self.a.as_mut() {
                        a[nz] = a[j];
                    }
                    mask[c] = nz as i32;
                    nz += 1;
                } else if let Some(a) = self.a.as_mut() {
                    let k = mask[c] as usize;
                    a[k] += a[j];
                }
            }
            sta = self.ia[i + 1];
            self.ia[i + 1] = nz as i32;
        }
        self.nz = nz;
        self.ja.truncate(nz);
        if let Some(a) = self.a.as_mut() {
            a.truncate(nz);
        }
    }
}

/// `distance(x, dim, i, j)`.
pub fn distance(x: &[f64], dim: usize, i: usize, j: usize) -> f64 {
    let mut d = 0.0f64;
    for k in 0..dim {
        let t = x[i * dim + k] - x[j * dim + k];
        d = math::fma(t, t, d);
    }
    d.sqrt()
}

/// `distance_cropped(x, dim, i, j)`.
pub fn distance_cropped(x: &[f64], dim: usize, i: usize, j: usize) -> f64 {
    distance(x, dim, i, j).max(MINDIST)
}

/// `vector_product(n, x, y)`.
pub fn vector_product(n: usize, x: &[f64], y: &[f64]) -> f64 {
    let mut r = 0.0f64;
    for i in 0..n {
        r = math::fma(x[i], y[i], r);
    }
    r
}

/// `drand()`: `rand() / RAND_MAX`.
pub fn drand() -> f64 {
    f64::from(crate::cutil::rand()) / f64::from(crate::cutil::RAND_MAX)
}
