// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Copyright (c) ZK-INFRA. All rights reserved.

//! Fixed-base Scalar Multiplication Gate

use crate::constraint_system::ecc::Point;
use crate::constraint_system::{variable::Variable, StandardComposer};
use ark_ec::models::twisted_edwards_extended::{GroupAffine, GroupProjective};
use ark_ec::models::TEModelParameters;
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_ff::{BigInteger, FpParameters, PrimeField};
use num_traits::{One, Zero};

fn compute_wnaf_point_multiples<P>(
    base_point: GroupProjective<P>,
) -> Vec<GroupAffine<P>>
where
    P: TEModelParameters,
    P::BaseField: PrimeField,
{
    let mut multiples = vec![
        GroupProjective::<P>::default();
        <P::BaseField as PrimeField>::Params::MODULUS_BITS
            as usize
    ];
    multiples[0] = base_point;
    for i in 1..<P::BaseField as PrimeField>::Params::MODULUS_BITS as usize {
        multiples[i] = multiples[i - 1].double();
    }
    ProjectiveCurve::batch_normalization_into_affine(&multiples)
}

impl<E, P> StandardComposer<E, P>
where
    E: PairingEngine,
    P: TEModelParameters<BaseField = E::Fr>,
{
    /// Adds an elliptic curve scalar multiplication gate to the circuit
    /// description.
    ///
    /// # Note
    ///
    /// This function is optimized for fixed base ops **ONLY** and therefore,
    /// the **ONLY** input that should be passed to the function as a point is
    /// the generator or basepoint of the curve over which we are operating.
    pub fn fixed_base_scalar_mul(
        &mut self,
        scalar: Variable,
        base_point: GroupAffine<P>,
    ) -> Point<E, P> {
        let num_bits =
            <P::BaseField as PrimeField>::Params::MODULUS_BITS as usize;
        // compute 2^iG
        let mut point_multiples =
            compute_wnaf_point_multiples(base_point.into());
        point_multiples.reverse();

        let scalar_value = self.variables.get(&scalar).unwrap();

        // Convert scalar to wnaf_2(k)
        let wnaf_entries =
            scalar_value.into_repr().find_wnaf(2).expect("Fix this!");
        // wnaf_entries.extend(vec![0i64; num_bits - wnaf_entries.len()]);
        assert!(wnaf_entries.len() <= num_bits);

        // Initialise the accumulators
        let mut scalar_acc = Vec::with_capacity(num_bits);
        scalar_acc.push(E::Fr::zero());
        let mut point_acc = Vec::with_capacity(num_bits);
        point_acc.push(GroupAffine::<P>::zero());

        // Auxillary point to help with checks on the backend
        let mut xy_alphas = Vec::with_capacity(num_bits);

        let n_trailing_zeros = num_bits - wnaf_entries.len();
        scalar_acc.extend(vec![E::Fr::zero(); n_trailing_zeros]);
        point_acc.extend(vec![GroupAffine::<P>::zero(); n_trailing_zeros]);
        xy_alphas.extend(vec![E::Fr::zero(); n_trailing_zeros]);

        // Load values into accumulators based on wnaf entries
        for (i, entry) in wnaf_entries.iter().rev().enumerate() {
            // Offset the index by the number of trailing zeros
            let index = i + n_trailing_zeros;
            // Based on the WNAF, we decide what scalar and point to add
            let (scalar_to_add, point_to_add) =
                match entry {
                    0 => { (E::Fr::zero(), GroupAffine::<P>::zero())},
                    -1 => {(-E::Fr::one(), -point_multiples[index])},
                    1 => {(E::Fr::one(), point_multiples[index])},
                    _ => unreachable!("Currently WNAF_2(k) is supported.
                        The possible values are 1, -1 and 0. Current entry is {}", entry),
                };

            let prev_accumulator = E::Fr::from(2u64) * scalar_acc[index];
            scalar_acc.push(prev_accumulator + scalar_to_add);
            point_acc.push(point_acc[index] + point_to_add);

            let x_alpha = point_to_add.x;
            let y_alpha = point_to_add.y;

            xy_alphas.push(x_alpha * y_alpha);
        }

        for i in 0..num_bits {
            let acc_x = self.add_input(point_acc[i].x);
            let acc_y = self.add_input(point_acc[i].y);

            let accumulated_bit = self.add_input(scalar_acc[i]);

            // We constrain the point accumulator to start from the Identity
            // point and the Scalar accumulator to start from zero
            if i == 0 {
                self.constrain_to_constant(acc_x, E::Fr::zero(), None);
                self.constrain_to_constant(acc_y, E::Fr::one(), None);
                self.constrain_to_constant(
                    accumulated_bit,
                    E::Fr::zero(),
                    None,
                );
            }

            let x_beta = point_multiples[i].x;
            let y_beta = point_multiples[i].y;

            let xy_alpha = self.add_input(xy_alphas[i]);

            let xy_beta = x_beta * y_beta;

            let wnaf_round = StandardComposer::<E, P>::new_wnaf(
                acc_x,
                acc_y,
                accumulated_bit,
                xy_alpha,
                x_beta,
                y_beta,
                xy_beta,
            );

            self.fixed_group_add(wnaf_round);
        }

        // Add last gate, but do not activate it for ECC
        // It is for use with the previous gate
        let acc_x = self.add_input(point_acc[num_bits].x);
        let acc_y = self.add_input(point_acc[num_bits].y);
        let xy_alpha = self.zero_var;
        let last_accumulated_bit = self.add_input(scalar_acc[num_bits]);

        self.big_add_gate(
            acc_x,
            acc_y,
            xy_alpha,
            Some(last_accumulated_bit),
            E::Fr::zero(),
            E::Fr::zero(),
            E::Fr::zero(),
            E::Fr::zero(),
            E::Fr::zero(),
            None,
        );

        // Constrain the last element in the accumulator to be equal to the
        // input scalar.
        self.assert_equal(last_accumulated_bit, scalar);

        Point::new(acc_x, acc_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{batch_test, constraint_system::helper::*, util};
    use ark_bls12_377::Bls12_377;
    use ark_bls12_381::Bls12_381;
    use ark_ec::{group::Group, AffineCurve};
    use ark_ff::PrimeField;

    fn test_ecc_constraint<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let scalar = E::Fr::from_le_bytes_mod_order(&[
                    182, 44, 247, 214, 94, 14, 151, 208, 130, 16, 200, 204,
                    147, 32, 104, 166, 0, 59, 52, 1, 1, 59, 103, 6, 169, 175,
                    51, 101, 234, 180, 125, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0,
                ]);
                let secret_scalar = composer.add_input(scalar);

                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let generator = GroupAffine::new(x, y);
                let expected_point: GroupAffine<P> = AffineCurve::mul(
                    &generator,
                    util::to_embedded_curve_scalar::<E, P>(scalar),
                )
                .into();

                let point_scalar =
                    composer.fixed_base_scalar_mul(secret_scalar, generator);

                composer
                    .assert_equal_public_point(point_scalar, expected_point);
            },
            600,
        );
        assert!(res.is_ok());
    }

    fn test_ecc_constraint_zero<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let scalar = E::Fr::zero();
                let secret_scalar = composer.add_input(scalar);

                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let generator = GroupAffine::new(x, y);
                let expected_point: GroupAffine<P> = AffineCurve::mul(
                    &generator,
                    util::to_embedded_curve_scalar::<E, P>(scalar),
                )
                .into();

                let point_scalar =
                    composer.fixed_base_scalar_mul(secret_scalar, generator);

                composer
                    .assert_equal_public_point(point_scalar, expected_point);
            },
            600,
        );
        assert!(res.is_ok());
    }

    fn test_ecc_constraint_should_fail<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let scalar = E::Fr::from(100u64);
                let secret_scalar = composer.add_input(scalar);
                // Fails because we are not multiplying by the GENERATOR, it is
                // double
                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let generator = GroupAffine::new(x, y);
                let double_gen = generator.double();

                let expected_point: GroupAffine<P> = AffineCurve::mul(
                    &double_gen,
                    util::to_embedded_curve_scalar::<E, P>(scalar),
                )
                .into();

                let point_scalar =
                    composer.fixed_base_scalar_mul(secret_scalar, generator);

                composer
                    .assert_equal_public_point(point_scalar, expected_point);
            },
            600,
        );

        assert!(res.is_err());
    }

    fn test_point_addition<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let generator = GroupAffine::new(x, y);

                let point_a = generator;
                let point_b = point_a.double();
                let expected_point = point_a + point_b;

                let affine_point_a = point_a;
                let affine_point_b = point_b;
                let affine_expected_point = expected_point;

                let var_point_a_x = composer.add_input(affine_point_a.x);
                let var_point_a_y = composer.add_input(affine_point_a.y);
                let point_a = Point::<E, P>::new(var_point_a_x, var_point_a_y);
                let var_point_b_x = composer.add_input(affine_point_b.x);
                let var_point_b_y = composer.add_input(affine_point_b.y);
                let point_b = Point::<E, P>::new(var_point_b_x, var_point_b_y);
                let new_point = composer.point_addition_gate(point_a, point_b);

                composer.assert_equal_public_point(
                    new_point,
                    affine_expected_point,
                );
            },
            600,
        );

        assert!(res.is_ok());
    }

    fn test_pedersen_hash<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let generator = GroupAffine::new(x, y);
                // First component
                let scalar_a = E::Fr::from(112233u64);
                let secret_scalar_a = composer.add_input(scalar_a);
                let point_a = generator;
                let expected_component_a: GroupAffine<P> = AffineCurve::mul(
                    &point_a,
                    util::to_embedded_curve_scalar::<E, P>(scalar_a),
                )
                .into();

                // Second component
                let scalar_b = E::Fr::from(445566u64);
                let secret_scalar_b = composer.add_input(scalar_b);
                let point_b = point_a.double() + point_a;
                let expected_component_b: GroupAffine<P> = AffineCurve::mul(
                    &point_b,
                    util::to_embedded_curve_scalar::<E, P>(scalar_b),
                )
                .into();

                // Expected pedersen hash
                let expected_point: GroupAffine<P> = (AffineCurve::mul(
                    &point_a,
                    util::to_embedded_curve_scalar::<E, P>(scalar_a),
                ) + AffineCurve::mul(
                    &point_b,
                    util::to_embedded_curve_scalar::<E, P>(scalar_b),
                ))
                .into();

                // To check this pedersen commitment, we will need to do:
                // - Two scalar multiplications
                // - One curve addition
                //
                // Scalar multiplications
                let component_a =
                    composer.fixed_base_scalar_mul(secret_scalar_a, point_a);
                let component_b =
                    composer.fixed_base_scalar_mul(secret_scalar_b, point_b);

                // Depending on the context, one can check if the resulting
                // components are as expected
                //
                composer.assert_equal_public_point(
                    component_a,
                    expected_component_a,
                );
                composer.assert_equal_public_point(
                    component_b,
                    expected_component_b,
                );

                // Curve addition
                let commitment =
                    composer.point_addition_gate(component_a, component_b);

                // Add final constraints to ensure that the commitment that we
                // computed is equal to the public point
                composer.assert_equal_public_point(commitment, expected_point);
            },
            1024,
        );
        assert!(res.is_ok());
    }

    fn test_pedersen_balance<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let generator = GroupAffine::new(x, y);

                // First component
                let scalar_a = E::Fr::from(25u64);
                let secret_scalar_a = composer.add_input(scalar_a);
                // Second component
                let scalar_b = E::Fr::from(30u64);
                let secret_scalar_b = composer.add_input(scalar_b);
                // Third component
                let scalar_c = E::Fr::from(10u64);
                let secret_scalar_c = composer.add_input(scalar_c);
                // Fourth component
                let scalar_d = E::Fr::from(45u64);
                let secret_scalar_d = composer.add_input(scalar_d);

                let (x, y) = P::AFFINE_GENERATOR_COEFFS;
                let gen: GroupAffine<P> = GroupAffine::new(x, y);

                let expected_lhs: GroupAffine<P> = AffineCurve::mul(
                    &gen,
                    util::to_embedded_curve_scalar::<E, P>(scalar_a + scalar_b),
                )
                .into();
                let expected_rhs: GroupAffine<P> = AffineCurve::mul(
                    &gen,
                    util::to_embedded_curve_scalar::<E, P>(scalar_c + scalar_d),
                )
                .into();

                let point_a =
                    composer.fixed_base_scalar_mul(secret_scalar_a, generator);
                let point_b =
                    composer.fixed_base_scalar_mul(secret_scalar_b, generator);
                let point_c =
                    composer.fixed_base_scalar_mul(secret_scalar_c, generator);
                let point_d =
                    composer.fixed_base_scalar_mul(secret_scalar_d, generator);

                let commitment_lhs =
                    composer.point_addition_gate(point_a, point_b);
                let commitment_rhs =
                    composer.point_addition_gate(point_c, point_d);

                composer.assert_equal_point(commitment_lhs, commitment_rhs);

                composer
                    .assert_equal_public_point(commitment_lhs, expected_lhs);
                composer
                    .assert_equal_public_point(commitment_rhs, expected_rhs);
            },
            2048,
        );
        assert!(res.is_ok());
    }

    // Bls12-381 tests
    batch_test!(
        [
            test_ecc_constraint,
            test_ecc_constraint_zero,
            test_ecc_constraint_should_fail,
            test_point_addition,
            test_pedersen_hash,
            test_pedersen_balance
        ],
        [] => (
            Bls12_381,
            ark_ed_on_bls12_381::EdwardsParameters
        )
    );

    // Bls12-377 tests
    batch_test!(
        [
            test_ecc_constraint,
            test_ecc_constraint_zero,
            test_ecc_constraint_should_fail,
            test_point_addition,
            test_pedersen_hash,
            test_pedersen_balance
        ],
        [] => (
            Bls12_377,
            ark_ed_on_bls12_377::EdwardsParameters
        )
    );
}
