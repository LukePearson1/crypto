// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Copyright (c) ZK-INFRA. All rights reserved.

//! This is an extension over the [Merlin Transcript](Transcript) which adds a
//! few extra functionalities.

use ark_ec::PairingEngine;
use ark_ff::{Field, PrimeField};
use ark_poly_commit::kzg10::Commitment;
use ark_serialize::CanonicalSerialize;
use core::marker::PhantomData;
use merlin::Transcript;

/// Wrapper around [`Transcript`]
#[derive(derivative::Derivative)]
#[derivative(Clone)]
pub struct TranscriptWrapper<E>
where
    E: PairingEngine,
{
    /// Base Transcript
    pub transcript: Transcript,

    /// Type Parameter Marker
    __: PhantomData<E>,
}

impl<E> TranscriptWrapper<E>
where
    E: PairingEngine,
{
    /// Builds a new [`TranscriptWrapper`] with the given `label`.
    #[inline]
    pub fn new(label: &'static [u8]) -> Self {
        Self {
            transcript: Transcript::new(label),
            __: PhantomData,
        }
    }
}

/// Transcript adds an abstraction over the Merlin transcript
/// For convenience
pub(crate) trait TranscriptProtocol<E>
where
    E: PairingEngine,
{
    /// Append a `commitment` with the given `label`.
    fn append_commitment(&mut self, label: &'static [u8], comm: &Commitment<E>);

    /// Append a scalar with the given `label`.
    fn append_scalar(&mut self, label: &'static [u8], s: &E::Fr);

    /// Compute a `label`ed challenge variable.
    fn challenge_scalar(&mut self, label: &'static [u8]) -> E::Fr;

    /// Append domain separator for the circuit size.
    fn circuit_domain_sep(&mut self, n: u64);
}

impl<E> TranscriptProtocol<E> for TranscriptWrapper<E>
where
    E: PairingEngine,
{
    fn append_commitment(
        &mut self,
        label: &'static [u8],
        comm: &Commitment<E>,
    ) {
        let mut bytes = Vec::new();
        comm.0.serialize(&mut bytes).unwrap();
        self.transcript.append_message(label, &bytes);
    }

    fn append_scalar(&mut self, label: &'static [u8], s: &E::Fr) {
        let mut bytes = Vec::new();
        s.serialize(&mut bytes).unwrap();
        self.transcript.append_message(label, &bytes)
    }

    fn challenge_scalar(&mut self, label: &'static [u8]) -> E::Fr {
        // XXX: review this: assure from_random_bytes returnes a valid Field
        // element
        let size = E::Fr::size_in_bits() / 8;
        let mut buf = vec![0u8; size];
        self.transcript.challenge_bytes(label, &mut buf);
        E::Fr::from_random_bytes(&buf).unwrap()
    }

    fn circuit_domain_sep(&mut self, n: u64) {
        self.transcript.append_message(b"dom-sep", b"circuit_size");
        self.transcript.append_u64(b"n", n);
    }
}
