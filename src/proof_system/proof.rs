// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Copyright (c) ZK-INFRA. All rights reserved.

//! A Proof stores the commitments to all of the elements that
//! are needed to univocally identify a prove of some statement.
//!
//! This module contains the implementation of the `StandardComposer`s
//! `Proof` structure and it's methods.

use crate::proof_system::ecc::CurveAddition;
use crate::proof_system::ecc::FixedBaseScalarMul;
use crate::proof_system::linearisation_poly::ProofEvaluations;
use crate::proof_system::logic::Logic;
use crate::proof_system::range::Range;
use crate::proof_system::GateConstraint;
use crate::proof_system::VerifierKey as PlonkVerifierKey;
use crate::transcript::TranscriptProtocol;
use crate::util;
use crate::util::EvaluationDomainExt;
use crate::{error::Error, transcript::TranscriptWrapper};
use ark_ec::{msm::VariableBaseMSM, AffineCurve, TEModelParameters};
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_ff::{fields::batch_inversion, Field, PrimeField};
use ark_poly::univariate::DensePolynomial;
use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
use ark_poly_commit::kzg10;
use ark_poly_commit::kzg10::{Commitment, VerifierKey, KZG10};
use ark_serialize::{
    CanonicalDeserialize, CanonicalSerialize, Read, SerializationError, Write,
};
use core::marker::PhantomData;
use rand_core::OsRng;

/// A Proof is a composition of `Commitment`s to the Witness, Permutation,
/// Quotient, Shifted and Opening polynomials as well as the
/// `ProofEvaluations`.
///
/// It's main goal is to allow the `Verifier` to
/// formally verify that the secret witnesses used to generate the [`Proof`]
/// satisfy a circuit that both [`Prover`](super::Prover) and
/// [`Verifier`](super::Verifier) have in common succintly and without any
/// capabilities of adquiring any kind of knowledge about the witness used to
/// construct the Proof.
#[derive(CanonicalDeserialize, CanonicalSerialize, derivative::Derivative)]
#[derivative(
    Clone(bound = ""),
    Debug(bound = ""),
    Default(bound = ""),
    Eq(bound = ""),
    PartialEq(bound = "")
)]
pub struct Proof<E, P>
where
    E: PairingEngine,
    P: TEModelParameters<BaseField = E::Fr>,
{
    /// Commitment to the witness polynomial for the left wires.
    pub(crate) a_comm: Commitment<E>,

    /// Commitment to the witness polynomial for the right wires.
    pub(crate) b_comm: Commitment<E>,

    /// Commitment to the witness polynomial for the output wires.
    pub(crate) c_comm: Commitment<E>,

    /// Commitment to the witness polynomial for the fourth wires.
    pub(crate) d_comm: Commitment<E>,

    /// Commitment to the permutation polynomial.
    pub(crate) z_comm: Commitment<E>,

    /// Commitment to the quotient polynomial.
    pub(crate) t_1_comm: Commitment<E>,

    /// Commitment to the quotient polynomial.
    pub(crate) t_2_comm: Commitment<E>,

    /// Commitment to the quotient polynomial.
    pub(crate) t_3_comm: Commitment<E>,

    /// Commitment to the quotient polynomial.
    pub(crate) t_4_comm: Commitment<E>,

    /// Commitment to the opening proof polynomial.
    pub(crate) w_z_comm: Commitment<E>,

    /// Commitment to the shifted opening proof polynomial.
    pub(crate) w_zw_comm: Commitment<E>,

    /// Subset of all of the evaluations added to the proof.
    pub(crate) evaluations: ProofEvaluations<E::Fr>,

    /// Type Parameter Marker
    pub(crate) __: PhantomData<P>,
}

impl<E, P> Proof<E, P>
where
    E: PairingEngine,
    P: TEModelParameters<BaseField = E::Fr>,
{
    /// Performs the verification of a [`Proof`] returning a boolean result.
    pub(crate) fn verify(
        &self,
        plonk_verifier_key: &PlonkVerifierKey<E, P>,
        transcript: &mut TranscriptWrapper<E>,
        verifier_key: &VerifierKey<E>,
        pub_inputs: &[E::Fr],
    ) -> Result<(), Error> {
        let domain =
            GeneralEvaluationDomain::<E::Fr>::new(plonk_verifier_key.n)
                .unwrap();

        // Subgroup checks are done when the proof is deserialised.

        // In order for the Verifier and Prover to have the same view in the
        // non-interactive setting Both parties must commit the same
        // elements into the transcript Below the verifier will simulate
        // an interaction with the prover by adding the same elements
        // that the prover added into the transcript, hence generating the
        // same challenges
        //
        // Add commitment to witness polynomials to transcript
        transcript.append_commitment(b"w_l", &self.a_comm);
        transcript.append_commitment(b"w_r", &self.b_comm);
        transcript.append_commitment(b"w_o", &self.c_comm);
        transcript.append_commitment(b"w_4", &self.d_comm);

        // Compute beta and gamma challenges
        let beta = transcript.challenge_scalar(b"beta");
        transcript.append_scalar(b"beta", &beta);
        let gamma = transcript.challenge_scalar(b"gamma");

        assert!(beta != gamma, "challenges must be different");

        // Add commitment to permutation polynomial to transcript
        transcript.append_commitment(b"z", &self.z_comm);

        // Compute quotient challenge
        let alpha = transcript.challenge_scalar(b"alpha");
        let range_sep_challenge =
            transcript.challenge_scalar(b"range separation challenge");
        let logic_sep_challenge =
            transcript.challenge_scalar(b"logic separation challenge");
        let fixed_base_sep_challenge =
            transcript.challenge_scalar(b"fixed base separation challenge");
        let var_base_sep_challenge =
            transcript.challenge_scalar(b"variable base separation challenge");

        // Add commitment to quotient polynomial to transcript
        transcript.append_commitment(b"t_1", &self.t_1_comm);
        transcript.append_commitment(b"t_2", &self.t_2_comm);
        transcript.append_commitment(b"t_3", &self.t_3_comm);
        transcript.append_commitment(b"t_4", &self.t_4_comm);

        // Compute evaluation point challenge
        let z_challenge = transcript.challenge_scalar(b"z");

        // Compute zero polynomial evaluated at `z_challenge`
        let z_h_eval = domain.evaluate_vanishing_polynomial(z_challenge);

        // Compute first lagrange polynomial evaluated at `z_challenge`
        let l1_eval =
            compute_first_lagrange_evaluation(&domain, &z_h_eval, &z_challenge);

        // Compute quotient polynomial evaluated at `z_challenge`
        let t_eval = self.compute_quotient_evaluation(
            &domain,
            pub_inputs,
            alpha,
            beta,
            gamma,
            z_challenge,
            z_h_eval,
            l1_eval,
            self.evaluations.permutation_eval,
        );

        // Compute commitment to quotient polynomial
        // This method is necessary as we pass the `un-splitted` variation
        // to our commitment scheme
        let t_comm =
            self.compute_quotient_commitment(&z_challenge, domain.size());

        // Add evaluations to transcript
        transcript.append_scalar(b"a_eval", &self.evaluations.a_eval);
        transcript.append_scalar(b"b_eval", &self.evaluations.b_eval);
        transcript.append_scalar(b"c_eval", &self.evaluations.c_eval);
        transcript.append_scalar(b"d_eval", &self.evaluations.d_eval);
        transcript.append_scalar(b"a_next_eval", &self.evaluations.a_next_eval);
        transcript.append_scalar(b"b_next_eval", &self.evaluations.b_next_eval);
        transcript.append_scalar(b"d_next_eval", &self.evaluations.d_next_eval);
        transcript
            .append_scalar(b"left_sig_eval", &self.evaluations.left_sigma_eval);
        transcript.append_scalar(
            b"right_sig_eval",
            &self.evaluations.right_sigma_eval,
        );
        transcript
            .append_scalar(b"out_sig_eval", &self.evaluations.out_sigma_eval);
        transcript
            .append_scalar(b"q_arith_eval", &self.evaluations.q_arith_eval);
        transcript.append_scalar(b"q_c_eval", &self.evaluations.q_c_eval);
        transcript.append_scalar(b"q_l_eval", &self.evaluations.q_l_eval);
        transcript.append_scalar(b"q_r_eval", &self.evaluations.q_r_eval);
        transcript
            .append_scalar(b"perm_eval", &self.evaluations.permutation_eval);
        transcript.append_scalar(b"t_eval", &t_eval);
        transcript.append_scalar(
            b"r_eval",
            &self.evaluations.linearisation_polynomial_eval,
        );

        // Compute linearisation commitment
        let r_comm = self.compute_linearisation_commitment(
            alpha,
            beta,
            gamma,
            range_sep_challenge,
            logic_sep_challenge,
            fixed_base_sep_challenge,
            var_base_sep_challenge,
            z_challenge,
            l1_eval,
            plonk_verifier_key,
        );

        // Commitment Scheme
        // Now we delegate computation to the commitment scheme by batch
        // checking two proofs.
        //
        // The `AggregateProof`, which proves that all the necessary
        // polynomials evaluated at `z_challenge` are
        // correct and a `Proof` which is proof that the
        // permutation polynomial evaluated at the shifted root of unity is
        // correct

        // Generation of the first aggregated proof: It ensures that the
        // polynomials evaluated at `z_challenge` are correct.

        // Reconstruct the Aggregated Proof commitments and evals
        // The proof consists of the witness commitment with no blinder
        let (aggregate_proof_commitment, aggregate_proof_eval) = self
            .gen_aggregate_proof(
                t_eval,
                t_comm,
                r_comm,
                plonk_verifier_key,
                transcript,
            );
        let aggregate_proof = kzg10::Proof {
            w: self.w_z_comm.0,
            random_v: None,
        };

        // Reconstruct the Aggregated Shift Proof commitments and evals
        // The proof consists of the witness commitment with no blinder
        let (aggregate_shift_proof_commitment, aggregate_shift_proof_eval) =
            self.gen_shift_aggregate_proof(transcript);

        let aggregate_shift_proof = kzg10::Proof {
            w: self.w_zw_comm.0,
            random_v: None,
        };

        // Add commitment to openings to transcript
        transcript.append_commitment(b"w_z", &self.w_z_comm);
        transcript.append_commitment(b"w_z_w", &self.w_zw_comm);

        let group_gen = domain.group_gen();

        match KZG10::<_, DensePolynomial<_>>::batch_check(
            verifier_key,
            &[aggregate_proof_commitment, aggregate_shift_proof_commitment],
            &[z_challenge, (z_challenge * group_gen)],
            &[aggregate_proof_eval, aggregate_shift_proof_eval],
            &[aggregate_proof, aggregate_shift_proof],
            &mut OsRng,
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::ProofVerificationError),
            Err(e) => panic!("{:?}", e),
        }
    }

    // TODO: Doc this
    fn gen_aggregate_proof(
        &self,
        t_eval: E::Fr,
        t_comm: Commitment<E>,
        r_comm: Commitment<E>,
        plonk_verifier_key: &PlonkVerifierKey<E, P>,
        transcript: &mut TranscriptWrapper<E>,
    ) -> (Commitment<E>, E::Fr) {
        let challenge = transcript.challenge_scalar(b"aggregate_witness");
        util::linear_combination(
            &[
                t_eval,
                self.evaluations.linearisation_polynomial_eval,
                self.evaluations.a_eval,
                self.evaluations.b_eval,
                self.evaluations.c_eval,
                self.evaluations.d_eval,
                self.evaluations.left_sigma_eval,
                self.evaluations.right_sigma_eval,
                self.evaluations.out_sigma_eval,
            ],
            &[
                t_comm,
                r_comm,
                self.a_comm,
                self.b_comm,
                self.c_comm,
                self.d_comm,
                plonk_verifier_key.permutation.left_sigma,
                plonk_verifier_key.permutation.right_sigma,
                plonk_verifier_key.permutation.out_sigma,
            ],
            challenge,
        )
    }

    // TODO: Doc this
    fn gen_shift_aggregate_proof(
        &self,
        transcript: &mut TranscriptWrapper<E>,
    ) -> (Commitment<E>, E::Fr) {
        let challenge = transcript.challenge_scalar(b"aggregate_witness");
        util::linear_combination(
            &[
                self.evaluations.permutation_eval,
                self.evaluations.a_next_eval,
                self.evaluations.b_next_eval,
                self.evaluations.d_next_eval,
            ],
            &[self.z_comm, self.a_comm, self.b_comm, self.d_comm],
            challenge,
        )
    }

    // TODO: Doc this
    fn compute_quotient_evaluation(
        &self,
        domain: &GeneralEvaluationDomain<E::Fr>,
        pub_inputs: &[E::Fr],
        alpha: E::Fr,
        beta: E::Fr,
        gamma: E::Fr,
        z_challenge: E::Fr,
        z_h_eval: E::Fr,
        l1_eval: E::Fr,
        z_hat_eval: E::Fr,
    ) -> E::Fr {
        // Compute the public input polynomial evaluated at `z_challenge`
        let pi_eval = compute_barycentric_eval(pub_inputs, z_challenge, domain);

        let alpha_sq = alpha.square();
        // r + PI(z)
        let a = self.evaluations.linearisation_polynomial_eval + pi_eval;

        // a + beta * sigma_1 + gamma
        let beta_sig1 = beta * self.evaluations.left_sigma_eval;
        let b_0 = self.evaluations.a_eval + beta_sig1 + gamma;

        // b+ beta * sigma_2 + gamma
        let beta_sig2 = beta * self.evaluations.right_sigma_eval;
        let b_1 = self.evaluations.b_eval + beta_sig2 + gamma;

        // c+ beta * sigma_3 + gamma
        let beta_sig3 = beta * self.evaluations.out_sigma_eval;
        let b_2 = self.evaluations.c_eval + beta_sig3 + gamma;

        // ((d + gamma) * z_hat) * alpha
        let b_3 = (self.evaluations.d_eval + gamma) * z_hat_eval * alpha;

        let b = b_0 * b_1 * b_2 * b_3;

        // l_1(z) * alpha^2
        let c = l1_eval * alpha_sq;

        // Return t_eval
        (a - b - c) * z_h_eval.inverse().unwrap()
    }

    /// Computes the quotient polynomial commitment at `z_challenge`.
    fn compute_quotient_commitment(
        &self,
        z_challenge: &E::Fr,
        n: usize,
    ) -> Commitment<E> {
        let n = n as u64;
        let z_n = z_challenge.pow(&[n, 0, 0, 0]);
        let z_two_n = z_challenge.pow(&[2 * n, 0, 0, 0]);
        let z_three_n = z_challenge.pow(&[3 * n, 0, 0, 0]);
        let t_comm = self.t_1_comm.0.into_projective()
            + self.t_2_comm.0.mul(z_n.into_repr())
            + self.t_3_comm.0.mul(z_two_n.into_repr())
            + self.t_4_comm.0.mul(z_three_n.into_repr());
        Commitment(t_comm.into_affine())
    }

    /// Computes the commitment to `[r]_1`.
    fn compute_linearisation_commitment(
        &self,
        alpha: E::Fr,
        beta: E::Fr,
        gamma: E::Fr,
        range_sep_challenge: E::Fr,
        logic_sep_challenge: E::Fr,
        fixed_base_sep_challenge: E::Fr,
        var_base_sep_challenge: E::Fr,
        z_challenge: E::Fr,
        l1_eval: E::Fr,
        plonk_verifier_key: &PlonkVerifierKey<E, P>,
    ) -> Commitment<E> {
        let mut scalars = Vec::with_capacity(6);
        let mut points = Vec::with_capacity(6);

        plonk_verifier_key
            .arithmetic
            .compute_linearisation_commitment(
                &mut scalars,
                &mut points,
                &self.evaluations,
            );

        Range::extend_linearisation_commitment::<E>(
            plonk_verifier_key.range_selector_commitment,
            range_sep_challenge,
            &self.evaluations,
            &mut scalars,
            &mut points,
        );

        Logic::extend_linearisation_commitment::<E>(
            plonk_verifier_key.logic_selector_commitment,
            logic_sep_challenge,
            &self.evaluations,
            &mut scalars,
            &mut points,
        );

        FixedBaseScalarMul::<_, P>::extend_linearisation_commitment::<E>(
            plonk_verifier_key.fixed_group_add_selector_commitment,
            fixed_base_sep_challenge,
            &self.evaluations,
            &mut scalars,
            &mut points,
        );

        CurveAddition::<_, P>::extend_linearisation_commitment::<E>(
            plonk_verifier_key.variable_group_add_selector_commitment,
            var_base_sep_challenge,
            &self.evaluations,
            &mut scalars,
            &mut points,
        );

        plonk_verifier_key
            .permutation
            .compute_linearisation_commitment(
                &mut scalars,
                &mut points,
                &self.evaluations,
                z_challenge,
                (alpha, beta, gamma),
                l1_eval,
                self.z_comm.0,
            );

        let scalars_repr =
            scalars.iter().map(E::Fr::into_repr).collect::<Vec<_>>();

        Commitment(
            VariableBaseMSM::multi_scalar_mul(&points, &scalars_repr).into(),
        )
    }
}

/// The first lagrange polynomial has the expression:
///
/// ```text
/// L_0(X) = mul_from_1_to_(n-1) [(X - omega^i) / (1 - omega^i)]
/// ```
///
/// with `omega` being the generator of the domain (the `n`th root of unity).
///
/// We use two equalities:
///   1. `mul_from_2_to_(n-1) [1 / (1 - omega^i)] = 1 / n`
///   2. `mul_from_2_to_(n-1) [(X - omega^i)] = (X^n - 1) / (X - 1)`
/// to obtain the expression:
///
/// ```text
/// L_0(X) = (X^n - 1) / n * (X - 1)
/// ```
fn compute_first_lagrange_evaluation<F>(
    domain: &GeneralEvaluationDomain<F>,
    z_h_eval: &F,
    z_challenge: &F,
) -> F
where
    F: PrimeField,
{
    let n_fr = F::from(domain.size() as u64);
    let denom = n_fr * (*z_challenge - F::one());
    *z_h_eval * denom.inverse().unwrap()
}

fn compute_barycentric_eval<F>(
    evaluations: &[F],
    point: F,
    domain: &GeneralEvaluationDomain<F>,
) -> F
where
    F: PrimeField,
{
    let numerator =
        domain.evaluate_vanishing_polynomial(point) * domain.size_inv();
    let range = 0..evaluations.len();

    let non_zero_evaluations = range
        .filter(|&i| {
            let evaluation = &evaluations[i];
            evaluation != &F::zero()
        })
        .collect::<Vec<_>>();

    // Only compute the denominators with non-zero evaluations
    let range = 0..non_zero_evaluations.len();

    let group_gen_inv = domain.group_gen_inv();
    let mut denominators = range
        .clone()
        .map(|i| {
            // index of non-zero evaluation
            let index = non_zero_evaluations[i];
            (group_gen_inv.pow(&[index as u64, 0, 0, 0]) * point) - F::one()
        })
        .collect::<Vec<_>>();
    batch_inversion(&mut denominators);

    let result: F = range
        .map(|i| {
            let eval_index = non_zero_evaluations[i];
            let eval = evaluations[eval_index];
            denominators[i] * eval
        })
        .sum();

    result * numerator
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::batch_test;
    use ark_bls12_377::Bls12_377;
    use ark_bls12_381::Bls12_381;
    use ark_ff::UniformRand;
    use rand_core::OsRng;

    fn test_serde_proof<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let proof = Proof::<E, P> {
            a_comm: Default::default(),
            b_comm: Default::default(),
            c_comm: Default::default(),
            d_comm: Default::default(),
            z_comm: Default::default(),
            t_1_comm: Default::default(),
            t_2_comm: Default::default(),
            t_3_comm: Default::default(),
            t_4_comm: Default::default(),
            w_z_comm: Default::default(),
            w_zw_comm: Default::default(),
            evaluations: ProofEvaluations {
                a_eval: E::Fr::rand(&mut OsRng),
                b_eval: E::Fr::rand(&mut OsRng),
                c_eval: E::Fr::rand(&mut OsRng),
                d_eval: E::Fr::rand(&mut OsRng),
                a_next_eval: E::Fr::rand(&mut OsRng),
                b_next_eval: E::Fr::rand(&mut OsRng),
                d_next_eval: E::Fr::rand(&mut OsRng),
                q_arith_eval: E::Fr::rand(&mut OsRng),
                q_c_eval: E::Fr::rand(&mut OsRng),
                q_l_eval: E::Fr::rand(&mut OsRng),
                q_r_eval: E::Fr::rand(&mut OsRng),
                left_sigma_eval: E::Fr::rand(&mut OsRng),
                right_sigma_eval: E::Fr::rand(&mut OsRng),
                out_sigma_eval: E::Fr::rand(&mut OsRng),
                linearisation_polynomial_eval: E::Fr::rand(&mut OsRng),
                permutation_eval: E::Fr::rand(&mut OsRng),
            },
            __: PhantomData,
        };

        let mut proof_bytes = vec![];
        proof.serialize(&mut proof_bytes).unwrap();

        let obtained_proof =
            Proof::deserialize(proof_bytes.as_slice()).unwrap();

        assert!(proof == obtained_proof);
    }

    // Bls12-381 tests
    batch_test!(
        [test_serde_proof],
        [] => (
            Bls12_381,
            ark_ed_on_bls12_381::EdwardsParameters
        )
    );

    // Bls12-377 tests
    batch_test!(
        [test_serde_proof],
        [] => (
            Bls12_377,
            ark_ed_on_bls12_377::EdwardsParameters
        )
    );
}
