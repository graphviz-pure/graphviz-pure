//! More of `lib/sparse/SparseMatrix.c`, used by sfdp's multilevel
//! coarsening: `SparseMatrix_multiply`, `SparseMatrix_multiply3`,
//! `SparseMatrix_divide_row_by_degree`, `SparseMatrix_has_diagonal`,
//! `SparseMatrix_decompose_to_supervariables`, and a CSR constructor.

use super::{Format, SparseMatrix};
use graphviz_pure_core::math;

impl SparseMatrix {
    /// A CSR matrix from its arrays (what C builds with `SparseMatrix_new`
    /// and then fills in place).
    pub fn from_csr(m: usize, n: usize, ia: Vec<i32>, ja: Vec<i32>, a: Option<Vec<f64>>) -> Self {
        let nz = ia[m] as usize;
        SparseMatrix {
            m,
            n,
            nz,
            ia,
            ja,
            a,
            format: Format::Csr,
            is_symmetric: false,
            is_pattern_symmetric: false,
        }
    }

    /// `SparseMatrix_has_diagonal(A)`.
    pub fn has_diagonal(&self) -> bool {
        for i in 0..self.m {
            for j in self.ia[i]..self.ia[i + 1] {
                if self.ja[j as usize] == i as i32 {
                    return true;
                }
            }
        }
        false
    }

    /// `SparseMatrix_divide_row_by_degree(A)` (in place).
    pub fn divide_row_by_degree(&mut self) {
        let Some(a) = self.a.as_mut() else { return };
        for i in 0..self.m {
            let deg = f64::from(self.ia[i + 1] - self.ia[i]);
            for j in self.ia[i]..self.ia[i + 1] {
                a[j as usize] /= deg;
            }
        }
    }

    /// `SparseMatrix_multiply(A, B)` for real matrices; `None` on a size mismatch.
    pub fn multiply(&self, b: &SparseMatrix) -> Option<SparseMatrix> {
        if self.n != b.m {
            return None;
        }
        let (ia, ja, ib, jb) = (&self.ia, &self.ja, &b.ia, &b.ja);
        let av = self.a.as_ref()?;
        let bv = b.a.as_ref()?;
        let m = self.m;
        let mut mask = vec![-1i32; b.n];
        let mut ic = vec![0i32; m + 1];
        let mut jc: Vec<i32> = Vec::new();
        let mut c: Vec<f64> = Vec::new();
        let mut nz = 0i32;
        for i in 0..m {
            for j in ia[i]..ia[i + 1] {
                let j = j as usize;
                let jj = ja[j] as usize;
                for k in ib[jj]..ib[jj + 1] {
                    let k = k as usize;
                    let col = jb[k] as usize;
                    if mask[col] < ic[i] {
                        mask[col] = nz;
                        jc.push(jb[k]);
                        c.push(av[j] * bv[k]);
                        nz += 1;
                    } else {
                        let t = mask[col] as usize;
                        c[t] = math::fma(av[j], bv[k], c[t]);
                    }
                }
            }
            ic[i + 1] = nz;
        }
        Some(SparseMatrix::from_csr(m, b.n, ic, jc, Some(c)))
    }

    /// `SparseMatrix_multiply3(A, B, C)` = A·B·C for real matrices.
    pub fn multiply3(&self, b: &SparseMatrix, c: &SparseMatrix) -> Option<SparseMatrix> {
        if self.n != b.m || b.n != c.m {
            return None;
        }
        let (ia, ja, ib, jb, ic, jc) = (&self.ia, &self.ja, &b.ia, &b.ja, &c.ia, &c.ja);
        let av = self.a.as_ref()?;
        let bv = b.a.as_ref()?;
        let cv = c.a.as_ref()?;
        let m = self.m;
        let mut mask = vec![-1i32; c.n];
        let mut id = vec![0i32; m + 1];
        let mut jd: Vec<i32> = Vec::new();
        let mut d: Vec<f64> = Vec::new();
        let mut nz = 0i32;
        for i in 0..m {
            for j in ia[i]..ia[i + 1] {
                let j = j as usize;
                let jj = ja[j] as usize;
                for l in ib[jj]..ib[jj + 1] {
                    let l = l as usize;
                    let ll = jb[l] as usize;
                    for k in ic[ll]..ic[ll + 1] {
                        let k = k as usize;
                        let col = jc[k] as usize;
                        if mask[col] < id[i] {
                            mask[col] = nz;
                            jd.push(jc[k]);
                            d.push(av[j] * bv[l] * cv[k]);
                            nz += 1;
                        } else {
                            let t = mask[col] as usize;
                            d[t] = math::fma(av[j] * bv[l], cv[k], d[t]);
                        }
                    }
                }
            }
            id[i + 1] = nz;
        }
        Some(SparseMatrix::from_csr(m, c.n, id, jd, Some(d)))
    }

    /// `SparseMatrix_decompose_to_supervariables(A, &ncluster, &cluster, &clusterp)`:
    /// groups columns with identical row patterns. Returns `(cluster, clusterp)`
    /// where cluster `i` is `cluster[clusterp[i]..clusterp[i + 1]]`.
    pub fn decompose_to_supervariables(&self) -> (Vec<i32>, Vec<i32>) {
        let (ia, ja) = (&self.ia, &self.ja);
        let n = self.n;
        let m = self.m;
        let mut sup = vec![0i32; n];
        // `nsuper` in C is offset by one so that `nsuper[-1]` exists.
        let mut nsuper_buf = vec![0i32; n + 1];
        let mut mask = vec![usize::MAX; n];
        let mut newmap = vec![0i32; n];
        let ns = |i: i32| (i + 1) as usize;
        let mut isup = 0i32;
        nsuper_buf[ns(0)] = n as i32;
        isup += 1;
        for i in 0..m {
            for j in ia[i]..ia[i + 1] {
                let isuper = sup[ja[j as usize] as usize];
                nsuper_buf[ns(isuper)] -= 1;
            }
            for j in ia[i]..ia[i + 1] {
                let col = ja[j as usize] as usize;
                let isuper = sup[col];
                let mk = mask[isuper as usize];
                if mk == usize::MAX || mk < i {
                    mask[isuper as usize] = i;
                    if nsuper_buf[ns(isuper)] == 0 {
                        nsuper_buf[ns(isuper)] = 1;
                        newmap[isuper as usize] = isuper;
                    } else {
                        newmap[isuper as usize] = isup;
                        nsuper_buf[ns(isup)] = 1;
                        sup[col] = isup;
                        isup += 1;
                    }
                } else {
                    sup[col] = newmap[isuper as usize];
                    nsuper_buf[ns(newmap[isuper as usize])] += 1;
                }
            }
        }
        // nsuper--: work on the unshifted array.
        let nsuper = &mut nsuper_buf;
        nsuper[0] = 0;
        for i in 0..isup as usize {
            nsuper[i + 1] += nsuper[i];
        }
        let mut cluster = newmap;
        for i in 0..n {
            let isuper = sup[i] as usize;
            cluster[nsuper[isuper] as usize] = i as i32;
            nsuper[isuper] += 1;
        }
        for i in (1..=isup as usize).rev() {
            nsuper[i] = nsuper[i - 1];
        }
        nsuper[0] = 0;
        nsuper.truncate(isup as usize + 1);
        (cluster, nsuper_buf)
    }
}
