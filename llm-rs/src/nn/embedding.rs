﻿use super::{NeuralNetwork, macros::*, unique};
use crate::{Blob, Context, Tensor};
use digit_layout::types;
use std::{iter::zip, ops::Add};
use tensor::rw_rc::RwRc;

pub struct Embedding {
    te: RwRc<Tensor<Blob>>,
    pe: RwRc<Tensor<Blob>>,
    tokens: Option<RwRc<Tensor<Blob>>>,
}

impl NeuralNetwork for Embedding {
    type Init = [RwRc<Tensor<Blob>>; 2];

    fn init(init: Self::Init, _ctx: &mut Context) -> Self {
        let [te, pe] = init;
        Self {
            te,
            pe,
            tokens: None,
        }
    }

    fn forward(
        &mut self,
        inputs: impl IntoIterator<Item = RwRc<Tensor<Blob>>>,
        _ctx: &mut Context,
    ) -> Vec<RwRc<Tensor<Blob>>> {
        destruct!([tokens] = inputs);
        self.tokens.replace(tokens);
        let Self { te, pe, tokens } = self;
        let te = te.read();
        let pe = pe.read();
        let tokens = tokens.as_ref().unwrap().read();

        dims!([batch_size, n_seq] = tokens);

        dims!([_, d] = te);
        let mut y = Tensor::new(te.dt(), &[batch_size, n_seq, d]).map(Blob::new);

        {
            let y = y.as_deref_mut().merge(0, 2);
            let i1 = tokens.as_deref().merge(0, 2);
            let i2 = Tensor::new(types::U16, &[batch_size * n_seq]).map(|len| {
                let mut blob = Blob::new(len);
                let ([], slice, []) = (unsafe { blob.align_to_mut::<u16>() }) else {
                    unreachable!()
                };
                for i in 0..batch_size {
                    for j in 0..n_seq {
                        slice[i * n_seq + j] = j as _
                    }
                }
                blob
            });
            forward::embedding(y, i1, i2.as_deref(), te.as_deref(), pe.as_deref())
        }

        vec![y.share()]
    }

    fn backward(
        &mut self,
        inputs: impl IntoIterator<Item = RwRc<Tensor<Blob>>>,
        ctx: &mut Context,
    ) -> Vec<RwRc<Tensor<Blob>>> {
        destruct!([dy] = inputs);
        let Self { te, pe, tokens } = self;

        {
            let i1 = tokens.take().unwrap();
            dims!([batch_size, n_seq] = i1.read());
            let i2 = Tensor::new(types::U16, &[batch_size * n_seq]).map(|len| {
                let mut blob = Blob::new(len);
                let ([], slice, []) = (unsafe { blob.align_to_mut::<u16>() }) else {
                    unreachable!()
                };
                for i in 0..batch_size {
                    for j in 0..n_seq {
                        slice[i * n_seq + j] = j as _
                    }
                }
                blob
            });
            backward::embedding(
                ctx.write_gradient("wte", te).write().as_deref_mut(),
                ctx.write_gradient("wpe", pe).write().as_deref_mut(),
                dy.read().as_deref().merge(0, 2),
                i1.read().as_deref().merge(0, 2),
                i2.as_deref(),
            )
        }

        te.release();
        pe.release();

        vec![]
    }
}

mod forward {
    use super::*;

    pub(super) fn embedding(
        mut y: Tensor<&mut [u8]>,
        i1: Tensor<&[u8]>,
        i2: Tensor<&[u8]>,
        table1: Tensor<&[u8]>,
        table2: Tensor<&[u8]>,
    ) {
        dims!([n0, d0] = y);
        dims!([n1] = i1);
        dims!([n2] = i2);
        dims!([_nt1, d1] = table1);
        dims!([_nt2, d2] = table2);

        let n = unique(&[n0, n1, n2]).unwrap();
        let d = unique(&[d0, d1, d2]).unwrap();

        strides!([nsy, dsy] = y);
        strides!([ns1] = i1);
        strides!([ns2] = i2);

        assert_eq!(dsy, y.dt().nbytes() as isize);
        assert_eq!(ns1, i1.dt().nbytes() as isize);
        assert_eq!(ns2, i2.dt().nbytes() as isize);
        assert!(table1.is_contiguous());
        assert!(table2.is_contiguous());

        let scheme = Scheme {
            n,
            d,
            nsy,
            y: y.mut_ptr(),
            i1: i1.ptr(),
            i2: i2.ptr(),
            table1: table1.ptr(),
            table2: table2.ptr(),
        };

        match (y.dt(), i1.dt(), i2.dt()) {
            (types::F32, types::U16, types::U16) => scheme.compute::<f32, u16, u16>(),
            (_, _, _) => todo!(),
        }
    }

    struct Scheme {
        n: usize,
        d: usize,
        nsy: isize,
        y: *mut u8,
        i1: *const u8,
        i2: *const u8,
        table1: *const u8,
        table2: *const u8,
    }

    trait Index: Copy + Sync {
        fn as_usize(self) -> usize;
    }

    impl<T: Copy + Sync + Into<usize>> Index for T {
        fn as_usize(self) -> usize {
            self.into()
        }
    }

    impl Scheme {
        fn compute<T: Add<Output = T>, I1: Index, I2: Index>(&self) {
            let &Self {
                n,
                d,
                nsy,
                y,
                i1,
                i2,
                table1,
                table2,
            } = self;
            let i1 = unsafe { std::slice::from_raw_parts(i1.cast::<I1>(), n) };
            let i2 = unsafe { std::slice::from_raw_parts(i2.cast::<I2>(), n) };
            for (i, (i1, i2)) in zip(i1, i2).enumerate() {
                let y = unsafe { y.byte_offset(nsy * i as isize) }.cast::<T>();
                let x1 = unsafe { table1.byte_add(i1.as_usize() * d * size_of::<T>()) }.cast::<T>();
                let x2 = unsafe { table2.byte_add(i2.as_usize() * d * size_of::<T>()) }.cast::<T>();
                for i in 0..d {
                    unsafe { y.add(i).write(x1.add(i).read() + x2.add(i).read()) }
                }
            }
        }
    }
}

mod backward {
    use std::ops::AddAssign;

    use super::*;

    pub(super) fn embedding(
        mut dtable1: Tensor<&mut [u8]>,
        mut dtable2: Tensor<&mut [u8]>,
        dy: Tensor<&[u8]>,
        i1: Tensor<&[u8]>,
        i2: Tensor<&[u8]>,
    ) {
        dims!([_nt1, d1] = dtable1);
        dims!([_nt2, d2] = dtable2);
        dims!([n0, d0] = dy);
        dims!([n1] = i1);
        dims!([n2] = i2);

        let n = unique(&[n0, n1, n2]).unwrap();
        let d = unique(&[d0, d1, d2]).unwrap();

        strides!([nsy, dsy] = dy);
        strides!([ns1] = i1);
        strides!([ns2] = i2);

        assert!(dtable1.is_contiguous());
        assert!(dtable2.is_contiguous());
        assert_eq!(dsy, dy.dt().nbytes() as isize);
        assert_eq!(ns1, i1.dt().nbytes() as isize);
        assert_eq!(ns2, i2.dt().nbytes() as isize);

        let scheme = Scheme {
            n,
            d,
            nsy,
            dtable1: dtable1.mut_ptr(),
            dtable2: dtable2.mut_ptr(),
            dy: dy.ptr(),
            i1: i1.ptr(),
            i2: i2.ptr(),
        };

        match (dy.dt(), i1.dt(), i2.dt()) {
            (types::F32, types::U16, types::U16) => scheme.compute::<f32, u16, u16>(),
            (_, _, _) => todo!(),
        }
    }

    struct Scheme {
        n: usize,
        d: usize,
        nsy: isize,
        dtable1: *mut u8,
        dtable2: *mut u8,
        dy: *const u8,
        i1: *const u8,
        i2: *const u8,
    }

    trait Index: Copy + Sync {
        fn as_usize(self) -> usize;
    }

    impl<T: Copy + Sync + Into<usize>> Index for T {
        fn as_usize(self) -> usize {
            self.into()
        }
    }

    impl Scheme {
        fn compute<T: AddAssign + Copy, I1: Index, I2: Index>(&self) {
            let &Self {
                n,
                d,
                nsy,
                dtable1,
                dtable2,
                dy,
                i1,
                i2,
            } = self;
            let i1 = unsafe { std::slice::from_raw_parts(i1.cast::<I1>(), n) };
            let i2 = unsafe { std::slice::from_raw_parts(i2.cast::<I2>(), n) };
            for (i, (i1, i2)) in zip(i1, i2).enumerate() {
                let dy = unsafe { dy.byte_offset(nsy * i as isize) }.cast::<T>();
                let x1 =
                    unsafe { dtable1.byte_add(i1.as_usize() * d * size_of::<T>()) }.cast::<T>();
                let x2 =
                    unsafe { dtable2.byte_add(i2.as_usize() * d * size_of::<T>()) }.cast::<T>();
                for i in 0..d {
                    unsafe { *x1.add(i) += *dy.add(i) }
                    unsafe { *x2.add(i) += *dy.add(i) }
                }
            }
        }
    }
}
