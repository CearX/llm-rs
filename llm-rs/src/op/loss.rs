use super::{Tensor, unique};
use crate::macros::*;
use digit_layout::types;
use std::iter::zip;

pub fn softmax(y: &Tensor, x: &Tensor, mask: usize) {
    clone_tensor!(y x);

    let dt = unique(&[y.dt(), x.dt()]).unwrap();
    assert_eq!(dt, types::F32);

    dims!([batch_size, n_seq, n_voc] = y);
    dims!([batch_size_, n_seq_, n_voc_] = x);
    assert_eq!(batch_size, batch_size_);
    assert_eq!(n_seq, n_seq_);
    assert_eq!(n_voc, n_voc_);

    for b in 0..batch_size {
        for t in 0..n_seq {
            let y = y
                .as_ref()
                .index(&[b, t])
                .map(|b| &mut **b.write())
                .vector_mut::<f32>();
            let x = x
                .as_ref()
                .index(&[b, t])
                .map(|b| &**b.read())
                .vector::<f32>();

            let (y, tail) = y.split_at_mut(mask);
            let x = &x[..mask];

            let max = x.iter().max_by(|a, b| f32::total_cmp(a, b)).unwrap();
            let mut expsum = 0.;
            for (y, &x) in zip(&mut *y, x) {
                *y = (x - max).exp();
                expsum += *y
            }

            for y in y {
                *y /= expsum
            }
            tail.fill(0.)
        }
    }
}

pub fn crossentropy(losses: &Tensor, probs: &Tensor, targets: &Tensor) {
    clone_tensor! {
        losses
        probs
    }

    assert_eq!(unique(&[losses.dt(), probs.dt()]).unwrap(), types::F32);
    assert_eq!(targets.dt(), types::U16);

    dims!([batch_size_0, n_seq_0] = losses);
    dims!([batch_size_1, n_seq_1, _] = probs);
    dims!([batch_size_2, n_seq_2] = targets);

    let batch_size = unique(&[batch_size_0, batch_size_1, batch_size_2]).unwrap();
    let n_seq = unique(&[n_seq_0, n_seq_1, n_seq_2]).unwrap();

    for b in 0..batch_size {
        for t in 0..n_seq {
            let losses = losses
                .as_ref()
                .index(&[b, t])
                .map(|b| &mut **b.write())
                .scalar_mut::<f32>();
            let probs = probs
                .as_ref()
                .index(&[b, t])
                .map(|b| &**b.read())
                .vector::<f32>();
            let target = targets
                .as_ref()
                .index(&[b, t])
                .map(|b| &**b.read())
                .scalar::<u16>();
            *losses = -probs[*target as usize].ln()
        }
    }
}

pub fn backward(dlogits: &Tensor, dlosses: &Tensor, probs: &Tensor, targets: &Tensor) {
    clone_tensor! {
        dlogits
        dlosses
        probs
        targets
    }

    let dt = unique(&[dlogits.dt(), dlosses.dt(), probs.dt()]).unwrap();
    assert_eq!(dt, types::F32);
    assert_eq!(targets.dt(), types::U16);

    dims!([batch_size_0, n_seq_0, n_voc_0] = dlogits);
    dims!([batch_size_1, n_seq_1] = dlosses);
    dims!([batch_size_2, n_seq_2, n_voc_1] = probs);
    dims!([batch_size_3, n_seq_3] = targets);

    let batch_size = unique(&[batch_size_0, batch_size_1, batch_size_2, batch_size_3]).unwrap();
    let n_seq = unique(&[n_seq_0, n_seq_1, n_seq_2, n_seq_3]).unwrap();
    let _ = unique(&[n_voc_0, n_voc_1]).unwrap();

    for b in 0..batch_size {
        for t in 0..n_seq {
            let dlogits = dlogits
                .as_ref()
                .index(&[b, t])
                .map(|b| &mut **b.write())
                .vector_mut::<f32>();
            let probs = probs
                .as_ref()
                .index(&[b, t])
                .map(|b| &**b.read())
                .vector::<f32>();
            let dloss = *dlosses
                .as_ref()
                .index(&[b, t])
                .map(|b| &**b.read())
                .scalar::<f32>();
            let ix = *targets
                .as_ref()
                .index(&[b, t])
                .map(|b| &**b.read())
                .scalar::<u16>() as usize;
            for (i, (dlogit, prob)) in zip(dlogits, probs).enumerate() {
                let indicator = if i == ix { 1. } else { 0. };
                *dlogit += (prob - indicator) * dloss
            }
        }
    }
}
