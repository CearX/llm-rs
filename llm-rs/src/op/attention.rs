﻿use super::{Tensor, unique};
use crate::macros::*;
use digit_layout::types;
use itertools::izip;
use std::{iter::zip, slice::from_raw_parts_mut};

pub fn forward(y: &Tensor, preatt: &Tensor, att: &Tensor, x: &Tensor) {
    clone_tensor!(y preatt att x);

    let dt = unique(&[y.dt(), preatt.dt(), att.dt(), x.dt()]).unwrap();
    assert_eq!(dt, types::F32);

    dims!([batch_size_0, n_seq_0, d] = y);
    dims!([batch_size_1, n_seq_1, d3] = x);
    dims!([batch_size_2, nh_0, n_seq_2, n_seq_3] = preatt);
    dims!([batch_size_3, nh_1, n_seq_4, n_seq_5] = att);

    let batch_size = unique(&[batch_size_0, batch_size_1, batch_size_2, batch_size_3]).unwrap();
    let n_seq = unique(&[n_seq_0, n_seq_1, n_seq_2, n_seq_3, n_seq_4, n_seq_5]).unwrap();
    let nh = unique(&[nh_0, nh_1]).unwrap();
    let d = unique(&[d, d3 / 3]).unwrap();
    let dh = d / nh;
    let scale = (dh as f32).powf(-0.5);

    for b in 0..batch_size {
        let qkv = x.as_ref().index(&[b]);
        for t in 0..n_seq {
            let q = qkv
                .as_deref()
                .index(&[t])
                .map(|b| &**b.read())
                .vector::<f32>();
            let y = y
                .as_ref()
                .index(&[b, t])
                .map(|b| &mut **b.write())
                .vector_mut::<f32>();

            for h in 0..nh {
                let y = &mut y[h * dh..][..dh];
                let q = &q[h * dh..][..dh];

                let preatt = preatt
                    .as_ref()
                    .index(&[b, h, t])
                    .map(|b| &mut **b.write())
                    .vector_mut::<f32>();
                let att = att
                    .as_ref()
                    .index(&[b, h, t])
                    .map(|b| &mut **b.write())
                    .vector_mut::<f32>();
                let (preatt, _) = preatt.split_at_mut(t + 1);
                let (att, tail) = att.split_at_mut(t + 1);

                // pass 1: calculate query dot key and maxval
                let mut max = f32::NEG_INFINITY;
                for (t_, val) in preatt.iter_mut().enumerate() {
                    let k = &qkv
                        .as_deref()
                        .index(&[t_])
                        .map(|b| &**b.read())
                        .vector::<f32>()[d..][h * dh..][..dh];
                    *val = zip(q, k).map(|(&q, &k)| q * k).sum::<f32>() * scale;
                    if *val > max {
                        max = *val
                    }
                }

                // pass 2: calculate the exp and keep track of sum
                let mut expsum = 0.;
                for (att, preatt) in zip(&mut *att, preatt) {
                    *att = (*preatt - max).exp();
                    expsum += *att
                }
                let expsum_inv = 1. / expsum;

                // pass 3: normalize to get the softmax
                for val in &mut *att {
                    *val *= expsum_inv
                }
                tail.fill(0.);

                // pass 4: accumulate weighted values into the output of attention
                y.fill(0.);
                for (t_, val) in att.iter_mut().enumerate() {
                    let v = &qkv
                        .as_ref()
                        .index(&[t_])
                        .map(|b| &**b.read())
                        .vector::<f32>()[d * 2..][h * dh..][..dh];
                    for (y, v) in zip(&mut *y, v) {
                        *y += *val * v
                    }
                }
            }
        }
    }
}

pub fn backward(
    dx: &Tensor,
    dpreatt: &Tensor,
    datt: &Tensor,
    dy: &Tensor,
    x: &Tensor,
    att: &Tensor,
) {
    clone_tensor!(dx dpreatt datt dy x att);

    let dt = unique(&[dx.dt(), dpreatt.dt(), datt.dt(), dy.dt(), x.dt(), att.dt()]).unwrap();
    assert_eq!(dt, types::F32);

    dims!([batch_size_0, n_seq_0, d3_0] = dx);
    dims!([batch_size_1, nh_0, n_seq_1, n_seq_2] = dpreatt);
    dims!([batch_size_2, nh_1, n_seq_3, n_seq_4] = datt);
    dims!([batch_size_3, n_seq_5, d_0] = dy);
    dims!([batch_size_4, n_seq_6, d3_1] = x);
    dims!([batch_size_5, nh_2, n_seq_7, n_seq_8] = att);

    let batch_size = unique(&[
        batch_size_0,
        batch_size_1,
        batch_size_2,
        batch_size_3,
        batch_size_4,
        batch_size_5,
    ])
    .unwrap();
    let n_seq = unique(&[
        n_seq_0, n_seq_1, n_seq_2, n_seq_3, n_seq_4, n_seq_5, n_seq_6, n_seq_7, n_seq_8,
    ])
    .unwrap();
    let nh = unique(&[nh_0, nh_1, nh_2]).unwrap();
    let d = unique(&[d3_0 / 3, d3_1 / 3, d_0]).unwrap();

    let dh = d / nh;
    let scale = (dh as f32).powf(-0.5);

    for b in 0..batch_size {
        for t in 0..n_seq {
            for h in 0..nh {
                let dx = dx.as_ref().index(&[b]);
                let x = x.as_ref().index(&[b]);

                let dpreatt = dpreatt
                    .as_ref()
                    .index(&[b, h, t])
                    .map(|b| &mut **b.write())
                    .vector_mut::<f32>();
                let datt = datt
                    .as_ref()
                    .index(&[b, h, t])
                    .map(|b| &mut **b.write())
                    .vector_mut::<f32>();
                let dy = &dy
                    .as_ref()
                    .index(&[b, t])
                    .map(|b| &**b.read())
                    .vector::<f32>()[h * dh..][..dh];
                let att = att
                    .as_ref()
                    .index(&[b, h, t])
                    .map(|b| &**b.read())
                    .vector::<f32>();

                for t_ in 0..=t {
                    let dqkv = dx
                        .as_ref()
                        .index(&[t_])
                        .map(|b| &mut **b.write())
                        .vector_mut::<f32>();
                    let qkv = x.as_ref().index(&[t_]).map(|b| &**b.read()).vector::<f32>();

                    let dv = &mut dqkv[2 * d..][h * dh..][..dh];
                    let v = &qkv[2 * d..][h * dh..][..dh];
                    let datt = &mut datt[t_];
                    let att = att[t_];

                    for (dv, v, dy) in izip!(&mut *dv, v, dy) {
                        *datt += v * dy;
                        *dv += att * dy;
                    }
                }
                for t_ in 0..=t {
                    for t__ in 0..=t {
                        let indicator = if t_ == t__ { 1. } else { 0. };
                        dpreatt[t__] += att[t_] * (indicator - att[t__]) * datt[t_];
                    }
                }

                let dqkv = dx
                    .as_ref()
                    .merge(0, 2)
                    .map(|b| &mut **b.write())
                    .vector_mut::<f32>();
                let qkv = x.as_ref().merge(0, 2).map(|b| &**b.read()).vector::<f32>();

                let dq =
                    unsafe { from_raw_parts_mut(dqkv[t * 3 * d..][h * dh..].as_mut_ptr(), dh) };
                let q = &qkv[t * 3 * d..][h * dh..][..dh];
                for t in 0..=t {
                    let dk = &mut dqkv[(t * 3 + 1) * d..][h * dh..][..dh];
                    let k = &qkv[(t * 3 + 1) * d..][h * dh..][..dh];
                    let dpreatt = dpreatt[t];

                    for (dq, q, dk, k) in izip!(&mut *dq, q, dk, k) {
                        *dq += k * dpreatt * scale;
                        *dk += q * dpreatt * scale;
                    }
                }
            }
        }
    }
}
