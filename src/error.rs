// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Copyright (c) ZK-INFRA. All rights reserved.

//! A collection of all possible errors encountered in PLONK.

/// Defines all possible errors that can be encountered in PLONK.
#[derive(Debug)]
pub enum Error {
    // FFT errors
    /// This error occurs when an error triggers on any of the fft module
    /// functions.
    InvalidEvalDomainSize {
        /// Log size of the group
        log_size_of_group: u32,
        /// Two adacity generated
        adacity: u32,
    },

    // Prover/Verifier errors
    /// This error occurs when a proof verification fails.
    ProofVerificationError,
    /// This error occurs when the circuit is not provided with all of the
    /// required inputs.
    CircuitInputsNotFound,
    /// This error occurs when we want to verify a Proof but the pi_constructor
    /// attribute is uninitialized.
    UninitializedPIGenerator,
    /// PublicInput serialization error
    InvalidPublicInputBytes,
    /// This error occurs when the Prover structure already contains a
    /// preprocessed circuit inside, but you call preprocess again.
    CircuitAlreadyPreprocessed,

    // Preprocessing errors
    /// This error occurs when an error triggers during the preprocessing
    /// stage.
    MismatchedPolyLen,

    /// Polynomial Commitment errors
    PCError {
        /// Polynomial Commitment errors
        error: ark_poly_commit::error::Error,
    },

    // KZG10 errors
    // XXX: Are these errors still used?
    /// This error occurs when the user tries to create PublicParameters
    /// and supplies the max degree as zero.
    DegreeIsZero,
    /// This error occurs when the user tries to trim PublicParameters
    /// to a degree that is larger than the maximum degree.
    TruncatedDegreeTooLarge,
    /// This error occurs when the user tries to trim PublicParameters
    /// down to a degree that is zero.
    TruncatedDegreeIsZero,
    /// This error occurs when the user tries to commit to a polynomial whose
    /// degree is larger than the supported degree for that proving key.
    PolynomialDegreeTooLarge,
    /// This error occurs when the user tries to commit to a polynomial whose
    /// degree is zero.
    PolynomialDegreeIsZero,
    /// This error occurs when the pairing check fails at being equal to the
    /// Identity point.
    PairingCheckFailure,

    /// This error occurs when there are not enough bytes to read out of a
    /// slice during deserialization.
    NotEnoughBytes,
    /// This error occurs when a malformed point is decoded from a byte array.
    PointMalformed,
    /// This error occurs when a malformed scalar is decoded from a byte
    /// array.
    ScalarMalformed,
}

impl From<ark_poly_commit::error::Error> for Error {
    fn from(error: ark_poly_commit::error::Error) -> Self {
        Self::PCError { error }
    }
}

#[cfg(feature = "std")]
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEvalDomainSize {
                log_size_of_group,
                adacity,
            } => write!(
                f,
                "Log-size of the EvaluationDomain group > TWO_ADACITY\
            Size: {:?} > TWO_ADACITY = {:?}",
                log_size_of_group, adacity
            ),
            Self::ProofVerificationError => {
                write!(f, "proof verification failed")
            }
            Self::CircuitInputsNotFound => {
                write!(f, "circuit inputs not found")
            }
            Self::UninitializedPIGenerator => {
                write!(f, "PI generator uninitialized")
            }
            Self::InvalidPublicInputBytes => {
                write!(f, "invalid public input bytes")
            }
            Self::MismatchedPolyLen => {
                write!(f, "the length of the wires is not the same")
            }
            Self::PCError { error } => {
                write!(f, "{:?}", error)
            }
            Self::CircuitAlreadyPreprocessed => {
                write!(f, "circuit has already been preprocessed")
            }
            Self::DegreeIsZero => {
                write!(f, "cannot create PublicParameters with max degree 0")
            }
            Self::TruncatedDegreeTooLarge => {
                write!(f, "cannot trim more than the maximum degree")
            }
            Self::TruncatedDegreeIsZero => write!(
                f,
                "cannot trim PublicParameters to a maximum size of zero"
            ),
            Self::PolynomialDegreeTooLarge => write!(
                f,
                "proving key is not large enough to commit to said polynomial"
            ),
            Self::PolynomialDegreeIsZero => {
                write!(f, "cannot commit to polynomial of zero degree")
            }
            Self::PairingCheckFailure => write!(f, "pairing check failed"),
            Self::NotEnoughBytes => write!(f, "not enough bytes left to read"),
            Self::PointMalformed => write!(f, "point bytes malformed"),
            Self::ScalarMalformed => write!(f, "scalar bytes malformed"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
