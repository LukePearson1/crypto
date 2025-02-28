// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Copyright (c) ZK-INFRA. All rights reserved.

//! Permutations

pub(crate) mod constants;

use crate::constraint_system::{Variable, WireData};
use ark_ff::PrimeField;
use ark_poly::domain::{EvaluationDomain, GeneralEvaluationDomain};
use ark_poly::{univariate::DensePolynomial, UVPolynomial};
use constants::*;
use core::marker::PhantomData;
use hashbrown::HashMap;
use itertools::izip;
use rand_core::RngCore;

/// Permutation provides the necessary state information and functions
/// to create the permutation polynomial. In the literature, Z(X) is the
/// "accumulator", this is what this codebase calls the permutation polynomial.
#[derive(derivative::Derivative)]
#[derivative(Debug)]
pub(crate) struct Permutation<F>
where
    F: PrimeField,
{
    /// Maps a variable to the wires that it is associated to.
    pub variable_map: HashMap<Variable, Vec<WireData>>,

    /// Type Parameter Marker
    __: PhantomData<F>,
}

impl<F> Permutation<F>
where
    F: PrimeField,
{
    /// Creates a Permutation struct with an expected capacity of zero.
    pub fn new() -> Self {
        Permutation::with_capacity(0)
    }

    /// Creates a Permutation struct with an expected capacity of `n`.
    pub fn with_capacity(expected_size: usize) -> Self {
        Self {
            variable_map: HashMap::with_capacity(expected_size),
            __: PhantomData,
        }
    }

    /// Creates a new [`Variable`] by incrementing the index of the
    /// `variable_map`. This is correct as whenever we add a new [`Variable`]
    /// into the system It is always allocated in the `variable_map`.
    pub fn new_variable(&mut self) -> Variable {
        // Generate the Variable
        let var = Variable(self.variable_map.keys().len());

        // Allocate space for the Variable on the variable_map
        // Each vector is initialised with a capacity of 16.
        // This number is a best guess estimate.
        self.variable_map.insert(var, Vec::with_capacity(16usize));

        var
    }

    /// Checks that the [`Variable`]s are valid by determining if they have been
    /// added to the system.
    fn valid_variables(&self, variables: &[Variable]) -> bool {
        variables
            .iter()
            .all(|var| self.variable_map.contains_key(var))
    }

    /// Maps a set of [`Variable`]s (a,b,c,d) to a set of [`Wire`](WireData)s
    /// (left, right, out, fourth) with the corresponding gate index
    pub fn add_variables_to_map(
        &mut self,
        a: Variable,
        b: Variable,
        c: Variable,
        d: Variable,
        gate_index: usize,
    ) {
        let left: WireData = WireData::Left(gate_index);
        let right: WireData = WireData::Right(gate_index);
        let output: WireData = WireData::Output(gate_index);
        let fourth: WireData = WireData::Fourth(gate_index);

        // Map each variable to the wire it is associated with
        // This essentially tells us that:
        self.add_variable_to_map(a, left);
        self.add_variable_to_map(b, right);
        self.add_variable_to_map(c, output);
        self.add_variable_to_map(d, fourth);
    }

    pub fn add_variable_to_map(&mut self, var: Variable, wire_data: WireData) {
        assert!(self.valid_variables(&[var]));

        // NOTE: Since we always allocate space for the Vec of WireData when a
        // `Variable` is added to the variable_map, this should never fail.
        let vec_wire_data = self.variable_map.get_mut(&var).unwrap();
        vec_wire_data.push(wire_data);
    }

    /// Performs shift by one permutation and computes `sigma_1`, `sigma_2` and
    /// `sigma_3`, `sigma_4` permutations from the variable maps.
    pub(super) fn compute_sigma_permutations(
        &mut self,
        n: usize,
    ) -> [Vec<WireData>; 4] {
        let sigma_1 = (0..n).map(WireData::Left).collect::<Vec<_>>();
        let sigma_2 = (0..n).map(WireData::Right).collect::<Vec<_>>();
        let sigma_3 = (0..n).map(WireData::Output).collect::<Vec<_>>();
        let sigma_4 = (0..n).map(WireData::Fourth).collect::<Vec<_>>();

        let mut sigmas = [sigma_1, sigma_2, sigma_3, sigma_4];

        for (_, wire_data) in self.variable_map.iter() {
            // Gets the data for each wire assosciated with this variable
            for (wire_index, current_wire) in wire_data.iter().enumerate() {
                // Fetch index of the next wire, if it is the last element
                // We loop back around to the beginning
                let next_index = match wire_index == wire_data.len() - 1 {
                    true => 0,
                    false => wire_index + 1,
                };

                // Fetch the next wire
                let next_wire = &wire_data[next_index];

                // Map current wire to next wire
                match current_wire {
                    WireData::Left(index) => sigmas[0][*index] = *next_wire,
                    WireData::Right(index) => sigmas[1][*index] = *next_wire,
                    WireData::Output(index) => sigmas[2][*index] = *next_wire,
                    WireData::Fourth(index) => sigmas[3][*index] = *next_wire,
                };
            }
        }

        sigmas
    }

    fn compute_permutation_lagrange(
        &self,
        sigma_mapping: &[WireData],
        domain: &GeneralEvaluationDomain<F>,
    ) -> Vec<F> {
        let roots: Vec<_> = domain.elements().collect();

        let lagrange_poly: Vec<F> = sigma_mapping
            .iter()
            .map(|x| match x {
                WireData::Left(index) => {
                    let root = &roots[*index];
                    *root
                }
                WireData::Right(index) => {
                    let root = &roots[*index];
                    K1::<F>() * root
                }
                WireData::Output(index) => {
                    let root = &roots[*index];
                    K2::<F>() * root
                }
                WireData::Fourth(index) => {
                    let root = &roots[*index];
                    K3::<F>() * root
                }
            })
            .collect();

        lagrange_poly
    }

    /// Computes the sigma polynomials which are used to build the permutation
    /// polynomial.
    pub fn compute_sigma_polynomials(
        &mut self,
        n: usize,
        domain: &GeneralEvaluationDomain<F>,
    ) -> (
        DensePolynomial<F>,
        DensePolynomial<F>,
        DensePolynomial<F>,
        DensePolynomial<F>,
    ) {
        // Compute sigma mappings
        let sigmas = self.compute_sigma_permutations(n);

        assert_eq!(sigmas[0].len(), n);
        assert_eq!(sigmas[1].len(), n);
        assert_eq!(sigmas[2].len(), n);
        assert_eq!(sigmas[3].len(), n);

        // define the sigma permutations using two non quadratic residues
        let left_sigma = self.compute_permutation_lagrange(&sigmas[0], domain);
        let right_sigma = self.compute_permutation_lagrange(&sigmas[1], domain);
        let out_sigma = self.compute_permutation_lagrange(&sigmas[2], domain);
        let fourth_sigma =
            self.compute_permutation_lagrange(&sigmas[3], domain);

        let left_sigma_poly =
            DensePolynomial::from_coefficients_vec(domain.ifft(&left_sigma));
        let right_sigma_poly =
            DensePolynomial::from_coefficients_vec(domain.ifft(&right_sigma));
        let out_sigma_poly =
            DensePolynomial::from_coefficients_vec(domain.ifft(&out_sigma));
        let fourth_sigma_poly =
            DensePolynomial::from_coefficients_vec(domain.ifft(&fourth_sigma));

        (
            left_sigma_poly,
            right_sigma_poly,
            out_sigma_poly,
            fourth_sigma_poly,
        )
    }

    #[allow(dead_code)]
    fn compute_slow_permutation_poly<I>(
        &self,
        domain: &GeneralEvaluationDomain<F>,
        w_l: I,
        w_r: I,
        w_o: I,
        w_4: I,
        beta: &F,
        gamma: &F,
        (left_sigma_poly, right_sigma_poly, out_sigma_poly, fourth_sigma_poly): (
            &DensePolynomial<F>,
            &DensePolynomial<F>,
            &DensePolynomial<F>,
            &DensePolynomial<F>,
        ),
    ) -> (Vec<F>, Vec<F>, Vec<F>)
    where
        I: Iterator<Item = F>,
    {
        let n = domain.size();

        let left_sigma_mapping = domain.fft(left_sigma_poly);
        let right_sigma_mapping = domain.fft(right_sigma_poly);
        let out_sigma_mapping = domain.fft(out_sigma_poly);
        let fourth_sigma_mapping = domain.fft(fourth_sigma_poly);

        // Compute beta * sigma polynomials
        let beta_left_sigma_iter =
            left_sigma_mapping.iter().map(|sigma| *sigma * beta);
        let beta_right_sigma_iter =
            right_sigma_mapping.iter().map(|sigma| *sigma * beta);
        let beta_out_sigma_iter =
            out_sigma_mapping.iter().map(|sigma| *sigma * beta);
        let beta_fourth_sigma_iter =
            fourth_sigma_mapping.iter().map(|sigma| *sigma * beta);

        // Compute beta * roots
        let beta_roots_iter = domain.elements().map(|root| root * beta);

        // Compute beta * roots * K1
        let beta_roots_k1_iter =
            domain.elements().map(|root| K1::<F>() * beta * root);

        // Compute beta * roots * K2
        let beta_roots_k2_iter =
            domain.elements().map(|root| K2::<F>() * beta * root);

        // Compute beta * roots * K3
        let beta_roots_k3_iter =
            domain.elements().map(|root| K3::<F>() * beta * root);

        // Compute left_wire + gamma
        let w_l_gamma: Vec<_> = w_l.map(|w| w + gamma).collect();

        // Compute right_wire + gamma
        let w_r_gamma: Vec<_> = w_r.map(|w| w + gamma).collect();

        // Compute out_wire + gamma
        let w_o_gamma: Vec<_> = w_o.map(|w| w + gamma).collect();

        // Compute fourth_wire + gamma
        let w_4_gamma: Vec<_> = w_4.map(|w| w + gamma).collect();

        let mut numerator_partial_components: Vec<F> = Vec::with_capacity(n);
        let mut denominator_partial_components: Vec<F> = Vec::with_capacity(n);

        let mut numerator_coefficients: Vec<F> = Vec::with_capacity(n);
        let mut denominator_coefficients: Vec<F> = Vec::with_capacity(n);

        // First element in both of them is one
        numerator_coefficients.push(F::one());
        denominator_coefficients.push(F::one());

        // Compute numerator coefficients
        for (
            w_l_gamma,
            w_r_gamma,
            w_o_gamma,
            w_4_gamma,
            beta_root,
            beta_root_k1,
            beta_root_k2,
            beta_root_k3,
        ) in izip!(
            w_l_gamma.iter(),
            w_r_gamma.iter(),
            w_o_gamma.iter(),
            w_4_gamma.iter(),
            beta_roots_iter,
            beta_roots_k1_iter,
            beta_roots_k2_iter,
            beta_roots_k3_iter,
        ) {
            // (w_l + beta * root + gamma)
            let prod_a = beta_root + w_l_gamma;

            // (w_r + beta * root * k_1 + gamma)
            let prod_b = beta_root_k1 + w_r_gamma;

            // (w_o + beta * root * k_2 + gamma)
            let prod_c = beta_root_k2 + w_o_gamma;

            // (w_4 + beta * root * k_3 + gamma)
            let prod_d = beta_root_k3 + w_4_gamma;

            let mut prod = prod_a * prod_b * prod_c * prod_d;

            numerator_partial_components.push(prod);

            prod *= numerator_coefficients.last().unwrap();

            numerator_coefficients.push(prod);
        }

        // Compute denominator coefficients
        for (
            w_l_gamma,
            w_r_gamma,
            w_o_gamma,
            w_4_gamma,
            beta_left_sigma,
            beta_right_sigma,
            beta_out_sigma,
            beta_fourth_sigma,
        ) in izip!(
            w_l_gamma,
            w_r_gamma,
            w_o_gamma,
            w_4_gamma,
            beta_left_sigma_iter,
            beta_right_sigma_iter,
            beta_out_sigma_iter,
            beta_fourth_sigma_iter,
        ) {
            // (w_l + beta * left_sigma + gamma)
            let prod_a = beta_left_sigma + w_l_gamma;

            // (w_r + beta * right_sigma + gamma)
            let prod_b = beta_right_sigma + w_r_gamma;

            // (w_o + beta * out_sigma + gamma)
            let prod_c = beta_out_sigma + w_o_gamma;

            // (w_4 + beta * fourth_sigma + gamma)
            let prod_d = beta_fourth_sigma + w_4_gamma;

            let mut prod = prod_a * prod_b * prod_c * prod_d;

            denominator_partial_components.push(prod);

            let last_element = denominator_coefficients.last().unwrap();

            prod *= last_element;

            denominator_coefficients.push(prod);
        }

        assert_eq!(denominator_coefficients.len(), n + 1);
        assert_eq!(numerator_coefficients.len(), n + 1);

        // Check that n+1'th elements are equal (taken from proof)
        let a = numerator_coefficients.last().unwrap();
        assert_ne!(a, &F::zero());
        let b = denominator_coefficients.last().unwrap();
        assert_ne!(b, &F::zero());
        assert_eq!(*a * b.inverse().unwrap(), F::one());

        // Remove those extra elements
        numerator_coefficients.remove(n);
        denominator_coefficients.remove(n);

        // Combine numerator and denominator

        let mut z_coefficients: Vec<F> = Vec::with_capacity(n);
        for (numerator, denominator) in numerator_coefficients
            .iter()
            .zip(denominator_coefficients.iter())
        {
            z_coefficients.push(*numerator * denominator.inverse().unwrap());
        }
        assert_eq!(z_coefficients.len(), n);

        (
            z_coefficients,
            numerator_partial_components,
            denominator_partial_components,
        )
    }

    #[allow(dead_code)]
    fn compute_fast_permutation_poly(
        &self,
        domain: &GeneralEvaluationDomain<F>,
        w_l: &[F],
        w_r: &[F],
        w_o: &[F],
        w_4: &[F],
        beta: F,
        gamma: F,
        (left_sigma_poly, right_sigma_poly, out_sigma_poly, fourth_sigma_poly): (
            &DensePolynomial<F>,
            &DensePolynomial<F>,
            &DensePolynomial<F>,
            &DensePolynomial<F>,
        ),
    ) -> Vec<F> {
        let n = domain.size();

        // Compute beta * roots
        let common_roots: Vec<F> =
            domain.elements().map(|root| root * beta).collect();

        let left_sigma_mapping = domain.fft(left_sigma_poly);
        let right_sigma_mapping = domain.fft(right_sigma_poly);
        let out_sigma_mapping = domain.fft(out_sigma_poly);
        let fourth_sigma_mapping = domain.fft(fourth_sigma_poly);

        // Compute beta * sigma polynomials
        let beta_left_sigmas: Vec<_> = left_sigma_mapping
            .iter()
            .copied()
            .map(|sigma| sigma * beta)
            .collect();
        let beta_right_sigmas: Vec<_> = right_sigma_mapping
            .iter()
            .copied()
            .map(|sigma| sigma * beta)
            .collect();
        let beta_out_sigmas: Vec<_> = out_sigma_mapping
            .iter()
            .copied()
            .map(|sigma| sigma * beta)
            .collect();
        let beta_fourth_sigmas: Vec<_> = fourth_sigma_mapping
            .iter()
            .copied()
            .map(|sigma| sigma * beta)
            .collect();

        // Compute beta * roots * K1
        let beta_roots_k1: Vec<_> = common_roots
            .iter()
            .copied()
            .map(|x| x * K1::<F>())
            .collect();

        // Compute beta * roots * K2
        let beta_roots_k2: Vec<_> = common_roots
            .iter()
            .copied()
            .map(|x| x * K2::<F>())
            .collect();

        // Compute beta * roots * K3
        let beta_roots_k3: Vec<_> = common_roots
            .iter()
            .copied()
            .map(|x| x * K3::<F>())
            .collect();

        // Compute left_wire + gamma
        let w_l_gamma: Vec<_> =
            w_l.iter().copied().map(|w_l| w_l + gamma).collect();

        // Compute right_wire + gamma
        let w_r_gamma: Vec<_> =
            w_r.iter().copied().map(|w_r| w_r + gamma).collect();

        // Compute out_wire + gamma
        let w_o_gamma: Vec<_> =
            w_o.iter().copied().map(|w_o| w_o + gamma).collect();

        // Compute fourth_wire + gamma
        let w_4_gamma: Vec<_> =
            w_4.iter().copied().map(|w_4| w_4 + gamma).collect();

        // Compute 6 accumulator components
        // Parallisable
        let accumulator_components_without_l1: Vec<_> = izip!(
            w_l_gamma,
            w_r_gamma,
            w_o_gamma,
            w_4_gamma,
            common_roots,
            beta_roots_k1,
            beta_roots_k2,
            beta_roots_k3,
            beta_left_sigmas,
            beta_right_sigmas,
            beta_out_sigmas,
            beta_fourth_sigmas,
        )
        .map(
            |(
                w_l_gamma,
                w_r_gamma,
                w_o_gamma,
                w_4_gamma,
                beta_root,
                beta_root_k1,
                beta_root_k2,
                beta_root_k3,
                beta_left_sigma,
                beta_right_sigma,
                beta_out_sigma,
                beta_fourth_sigma,
            )| {
                // w_j + beta * root^j-1 + gamma
                let ac1 = w_l_gamma + beta_root;

                // w_{n+j} + beta * K1 * root^j-1 + gamma
                let ac2 = w_r_gamma + beta_root_k1;

                // w_{2n+j} + beta * K2 * root^j-1 + gamma
                let ac3 = w_o_gamma + beta_root_k2;

                // w_{3n+j} + beta * K3 * root^j-1 + gamma
                let ac4 = w_4_gamma + beta_root_k3;

                // 1 / w_j + beta * sigma(j) + gamma
                let ac5 = (w_l_gamma + beta_left_sigma).inverse().unwrap();

                // 1 / w_{n+j} + beta * sigma(n+j) + gamma
                let ac6 = (w_r_gamma + beta_right_sigma).inverse().unwrap();

                // 1 / w_{2n+j} + beta * sigma(2n+j) + gamma
                let ac7 = (w_o_gamma + beta_out_sigma).inverse().unwrap();

                // 1 / w_{3n+j} + beta * sigma(3n+j) + gamma
                let ac8 = (w_4_gamma + beta_fourth_sigma).inverse().unwrap();

                (ac1, ac2, ac3, ac4, ac5, ac6, ac7, ac8)
            },
        )
        .collect();

        // Prepend ones to the beginning of each accumulator to signify L_1(x)
        let accumulator_components = core::iter::once((
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
        ))
        .chain(accumulator_components_without_l1);

        // Multiply each component of the accumulators
        // A simplified example is the following:
        // A1 = [1,2,3,4]
        // result = [1, 1*2, 1*2*3, 1*2*3*4]
        // Non Parallelisable
        let mut prev = (
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
            F::one(),
        );
        let product_acumulated_components: Vec<_> = accumulator_components
            .map(move |current_component| {
                prev.0 *= current_component.0;
                prev.1 *= current_component.1;
                prev.2 *= current_component.2;
                prev.3 *= current_component.3;
                prev.4 *= current_component.4;
                prev.5 *= current_component.5;
                prev.6 *= current_component.6;
                prev.7 *= current_component.7;

                prev
            })
            .collect();

        // Right now we basically have 6 acumulators of the form:
        // A1 = [a1, a1 * a2, a1*a2*a3,...]
        // A2 = [b1, b1 * b2, b1*b2*b3,...]
        // A3 = [c1, c1 * c2, c1*c2*c3,...]
        // ... and so on
        // We want:
        // [a1*b1*c1, a1 * a2 *b1 * b2 * c1 * c2,...]
        // Parallisable
        let mut z: Vec<_> = product_acumulated_components
            .iter()
            .map(move |current_component| {
                let mut prev = F::one();
                prev *= current_component.0;
                prev *= current_component.1;
                prev *= current_component.2;
                prev *= current_component.3;
                prev *= current_component.4;
                prev *= current_component.5;
                prev *= current_component.6;
                prev *= current_component.7;

                prev
            })
            .collect();
        // Remove the last(n+1'th) element
        z.remove(n);

        assert_eq!(n, z.len());

        z
    }

    // These are the formulas for the irreducible factors used in the product
    // argument
    fn numerator_irreducible(root: F, w: F, k: F, beta: F, gamma: F) -> F {
        w + beta * k * root + gamma
    }

    fn denominator_irreducible(
        _root: F,
        w: F,
        sigma: F,
        beta: F,
        gamma: F,
    ) -> F {
        w + beta * sigma + gamma
    }

    // This can be adapted into a general product argument
    // for any number of wires, with specific formulas defined
    // in the numerator_irreducible and denominator_irreducible functions
    pub fn compute_permutation_poly(
        &self,
        domain: &GeneralEvaluationDomain<F>,
        wires: (&[F], &[F], &[F], &[F]),
        beta: F,
        gamma: F,
        sigma_polys: (
            &DensePolynomial<F>,
            &DensePolynomial<F>,
            &DensePolynomial<F>,
            &DensePolynomial<F>,
        ),
    ) -> DensePolynomial<F> {
        let n = domain.size();

        // Constants defining cosets H, k1H, k2H, etc
        let ks = vec![F::one(), K1::<F>(), K2::<F>(), K3::<F>()];

        let sigma_mappings = (
            domain.fft(sigma_polys.0),
            domain.fft(sigma_polys.1),
            domain.fft(sigma_polys.2),
            domain.fft(sigma_polys.3),
        );

        // Transpose wires and sigma values to get "rows" in the form [wl_i,
        // wr_i, wo_i, ... ] where each row contains the wire and sigma
        // values for a single gate
        let gatewise_wires = izip!(wires.0, wires.1, wires.2, wires.3)
            .map(|(w0, w1, w2, w3)| vec![w0, w1, w2, w3]);
        let gatewise_sigmas = izip!(
            sigma_mappings.0,
            sigma_mappings.1,
            sigma_mappings.2,
            sigma_mappings.3
        )
        .map(|(s0, s1, s2, s3)| vec![s0, s1, s2, s3]);

        // Compute all roots
        // Non-parallelizable?
        let roots: Vec<F> = domain.elements().collect();

        let product_argument = izip!(roots, gatewise_sigmas, gatewise_wires)
            // Associate each wire value in a gate with the k defining its coset
            .map(|(gate_root, gate_sigmas, gate_wires)| {
                (gate_root, izip!(gate_sigmas, gate_wires, &ks))
            })
            // Now the ith element represents gate i and will have the form:
            //   (root_i, ((w0_i, s0_i, k0), (w1_i, s1_i, k1), ..., (wm_i, sm_i,
            // km)))   for m different wires, which is all the
            // information   needed for a single product coefficient
            // for a single gate Multiply up the numerator and
            // denominator irreducibles for each gate   and pair the
            // results
            .map(|(gate_root, wire_params)| {
                (
                    // Numerator product
                    wire_params
                        .clone()
                        .map(|(_sigma, wire, k)| {
                            Permutation::numerator_irreducible(
                                gate_root, *wire, *k, beta, gamma,
                            )
                        })
                        .product::<F>(),
                    // Denominator product
                    wire_params
                        .map(|(sigma, wire, _k)| {
                            Permutation::denominator_irreducible(
                                gate_root, *wire, sigma, beta, gamma,
                            )
                        })
                        .product::<F>(),
                )
            })
            // Divide each pair to get the single scalar representing each gate
            .map(|(n, d)| n * d.inverse().unwrap())
            // Collect into vector intermediary since rayon does not support
            // `scan`
            .collect::<Vec<F>>();

        let mut z = Vec::with_capacity(n);

        // First element is one
        let mut state = F::one();
        z.push(state);

        // Accumulate by successively multiplying the scalars
        // Non-parallelizable?
        for s in product_argument {
            state *= s;
            z.push(state);
        }

        // Remove the last(n+1'th) element
        z.remove(n);

        assert_eq!(n, z.len());

        DensePolynomial::<F>::from_coefficients_vec(domain.ifft(&z))
    }
}

/// The `bls_12-381` library does not provide a `random` method for `F`.
/// We wil use this helper function to compensate.
#[allow(dead_code)]
pub(crate) fn random_scalar<F: PrimeField, R: RngCore>(rng: &mut R) -> F {
    F::rand(rng)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::batch_test;
    use crate::{
        constraint_system::StandardComposer, util::EvaluationDomainExt,
    };
    use ark_bls12_377::Bls12_377;
    use ark_bls12_381::Bls12_381;
    use ark_ff::Field;
    use ark_ff::UniformRand;
    use ark_poly::univariate::DensePolynomial;
    use ark_poly::Polynomial;
    use num_traits::{One, Zero};
    // use rand::{rngs::StdRng, SeedableRng};
    use ark_ec::{PairingEngine, TEModelParameters};
    use rand_core::OsRng;

    fn test_multizip_permutation_poly<
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    >() {
        let mut cs: StandardComposer<E, P> =
            StandardComposer::with_expected_size(4);

        let zero = E::Fr::zero();
        let one = E::Fr::one();
        let two = one + one;

        let x1 = cs.add_input(E::Fr::from(4u64));
        let x2 = cs.add_input(E::Fr::from(12u64));
        let x3 = cs.add_input(E::Fr::from(8u64));
        let x4 = cs.add_input(E::Fr::from(3u64));

        // x1 * x4 = x2
        cs.poly_gate(x1, x4, x2, one, zero, zero, -one, zero, None);

        // x1 + x3 = x2
        cs.poly_gate(x1, x3, x2, zero, one, one, -one, zero, None);

        // x1 + x2 = 2*x3
        cs.poly_gate(x1, x2, x3, zero, one, one, -two, zero, None);

        // x3 * x4 = 2*x2
        cs.poly_gate(x3, x4, x2, one, zero, zero, -two, zero, None);

        let domain =
            GeneralEvaluationDomain::<E::Fr>::new(cs.circuit_size()).unwrap();
        let pad = vec![E::Fr::zero(); domain.size() - cs.w_l.len()];
        let mut w_l_scalar: Vec<E::Fr> =
            cs.w_l.iter().map(|v| cs.variables[v]).collect();
        let mut w_r_scalar: Vec<E::Fr> =
            cs.w_r.iter().map(|v| cs.variables[v]).collect();
        let mut w_o_scalar: Vec<E::Fr> =
            cs.w_o.iter().map(|v| cs.variables[v]).collect();
        let mut w_4_scalar: Vec<E::Fr> =
            cs.w_4.iter().map(|v| cs.variables[v]).collect();

        w_l_scalar.extend(&pad);
        w_r_scalar.extend(&pad);
        w_o_scalar.extend(&pad);
        w_4_scalar.extend(&pad);

        let sigmas: Vec<Vec<E::Fr>> = cs
            .perm
            .compute_sigma_permutations(7)
            .iter()
            .map(|wd| cs.perm.compute_permutation_lagrange(wd, &domain))
            .collect();

        let beta = E::Fr::rand(&mut OsRng);
        let gamma = E::Fr::rand(&mut OsRng);

        let sigma_polys: Vec<DensePolynomial<E::Fr>> = sigmas
            .iter()
            .map(|v| DensePolynomial::from_coefficients_vec(domain.ifft(v)))
            .collect();

        let mz = cs.perm.compute_permutation_poly(
            &domain,
            (&w_l_scalar, &w_r_scalar, &w_o_scalar, &w_4_scalar),
            beta,
            gamma,
            (
                &sigma_polys[0],
                &sigma_polys[1],
                &sigma_polys[2],
                &sigma_polys[3],
            ),
        );

        let old_z = DensePolynomial::from_coefficients_vec(domain.ifft(
            &cs.perm.compute_fast_permutation_poly(
                &domain,
                &w_l_scalar,
                &w_r_scalar,
                &w_o_scalar,
                &w_4_scalar,
                beta,
                gamma,
                (
                    &sigma_polys[0],
                    &sigma_polys[1],
                    &sigma_polys[2],
                    &sigma_polys[3],
                ),
            ),
        ));

        assert!(mz == old_z);
    }

    fn test_permutation_format<
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    >() {
        let mut perm: Permutation<E::Fr> = Permutation::new();

        let num_variables = 10u8;
        for i in 0..num_variables {
            let var = perm.new_variable();
            assert_eq!(var.0, i as usize);
            assert_eq!(perm.variable_map.len(), (i as usize) + 1);
        }

        let var_one = perm.new_variable();
        let var_two = perm.new_variable();
        let var_three = perm.new_variable();

        let gate_size = 100;
        for i in 0..gate_size {
            perm.add_variables_to_map(var_one, var_one, var_two, var_three, i);
        }

        // Check all gate_indices are valid
        for (_, wire_data) in perm.variable_map.iter() {
            for wire in wire_data.iter() {
                match wire {
                    WireData::Left(index)
                    | WireData::Right(index)
                    | WireData::Output(index)
                    | WireData::Fourth(index) => assert!(*index < gate_size),
                };
            }
        }
    }

    fn test_permutation_compute_sigmas_only_left_wires<
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    >() {
        let mut perm = Permutation::<E::Fr>::new();

        let var_zero = perm.new_variable();
        let var_two = perm.new_variable();
        let var_three = perm.new_variable();
        let var_four = perm.new_variable();
        let var_five = perm.new_variable();
        let var_six = perm.new_variable();
        let var_seven = perm.new_variable();
        let var_eight = perm.new_variable();
        let var_nine = perm.new_variable();

        let num_wire_mappings = 4;

        // Add four wire mappings
        perm.add_variables_to_map(var_zero, var_zero, var_five, var_nine, 0);
        perm.add_variables_to_map(var_zero, var_two, var_six, var_nine, 1);
        perm.add_variables_to_map(var_zero, var_three, var_seven, var_nine, 2);
        perm.add_variables_to_map(var_zero, var_four, var_eight, var_nine, 3);

        /*
        var_zero = {L0, R0, L1, L2, L3}
        var_two = {R1}
        var_three = {R2}
        var_four = {R3}
        var_five = {O0}
        var_six = {O1}
        var_seven = {O2}
        var_eight = {O3}
        var_nine = {F0, F1, F2, F3}
        Left_sigma = {R0, L2, L3, L0}
        Right_sigma = {L1, R1, R2, R3}
        Out_sigma = {O0, O1, O2, O3}
        Fourth_sigma = {F1, F2, F3, F0}
        */
        let sigmas = perm.compute_sigma_permutations(num_wire_mappings);
        let left_sigma = &sigmas[0];
        let right_sigma = &sigmas[1];
        let out_sigma = &sigmas[2];
        let fourth_sigma = &sigmas[3];

        // Check the left sigma polynomial
        assert_eq!(left_sigma[0], WireData::Right(0));
        assert_eq!(left_sigma[1], WireData::Left(2));
        assert_eq!(left_sigma[2], WireData::Left(3));
        assert_eq!(left_sigma[3], WireData::Left(0));

        // Check the right sigma polynomial
        assert_eq!(right_sigma[0], WireData::Left(1));
        assert_eq!(right_sigma[1], WireData::Right(1));
        assert_eq!(right_sigma[2], WireData::Right(2));
        assert_eq!(right_sigma[3], WireData::Right(3));

        // Check the output sigma polynomial
        assert_eq!(out_sigma[0], WireData::Output(0));
        assert_eq!(out_sigma[1], WireData::Output(1));
        assert_eq!(out_sigma[2], WireData::Output(2));
        assert_eq!(out_sigma[3], WireData::Output(3));

        // Check the output sigma polynomial
        assert_eq!(fourth_sigma[0], WireData::Fourth(1));
        assert_eq!(fourth_sigma[1], WireData::Fourth(2));
        assert_eq!(fourth_sigma[2], WireData::Fourth(3));
        assert_eq!(fourth_sigma[3], WireData::Fourth(0));

        let domain =
            GeneralEvaluationDomain::<E::Fr>::new(num_wire_mappings).unwrap();
        let w = domain.group_gen();
        let w_squared = w.pow(&[2, 0, 0, 0]);
        let w_cubed = w.pow(&[3, 0, 0, 0]);

        // Check the left sigmas have been encoded properly
        // Left_sigma = {R0, L2, L3, L0}
        // Should turn into {1 * K1, w^2, w^3, 1}
        let encoded_left_sigma =
            perm.compute_permutation_lagrange(left_sigma, &domain);
        assert_eq!(encoded_left_sigma[0], E::Fr::one() * K1::<E::Fr>());
        assert_eq!(encoded_left_sigma[1], w_squared);
        assert_eq!(encoded_left_sigma[2], w_cubed);
        assert_eq!(encoded_left_sigma[3], E::Fr::one());

        // Check the right sigmas have been encoded properly
        // Right_sigma = {L1, R1, R2, R3}
        // Should turn into {w, w * K1, w^2 * K1, w^3 * K1}
        let encoded_right_sigma =
            perm.compute_permutation_lagrange(right_sigma, &domain);
        assert_eq!(encoded_right_sigma[0], w);
        assert_eq!(encoded_right_sigma[1], w * K1::<E::Fr>());
        assert_eq!(encoded_right_sigma[2], w_squared * K1::<E::Fr>());
        assert_eq!(encoded_right_sigma[3], w_cubed * K1::<E::Fr>());

        // Check the output sigmas have been encoded properly
        // Out_sigma = {O0, O1, O2, O3}
        // Should turn into {1 * K2, w * K2, w^2 * K2, w^3 * K2}

        let encoded_output_sigma =
            perm.compute_permutation_lagrange(out_sigma, &domain);
        assert_eq!(encoded_output_sigma[0], E::Fr::one() * K2::<E::Fr>());
        assert_eq!(encoded_output_sigma[1], w * K2::<E::Fr>());
        assert_eq!(encoded_output_sigma[2], w_squared * K2::<E::Fr>());
        assert_eq!(encoded_output_sigma[3], w_cubed * K2::<E::Fr>());

        // Check the fourth sigmas have been encoded properly
        // Out_sigma = {F1, F2, F3, F0}
        // Should turn into {w * K3, w^2 * K3, w^3 * K3, 1 * K3}
        let encoded_fourth_sigma =
            perm.compute_permutation_lagrange(fourth_sigma, &domain);
        assert_eq!(encoded_fourth_sigma[0], w * K3::<E::Fr>());
        assert_eq!(encoded_fourth_sigma[1], w_squared * K3::<E::Fr>());
        assert_eq!(encoded_fourth_sigma[2], w_cubed * K3::<E::Fr>());
        assert_eq!(encoded_fourth_sigma[3], K3());

        let w_l = vec![
            E::Fr::from(2u64),
            E::Fr::from(2u64),
            E::Fr::from(2u64),
            E::Fr::from(2u64),
        ];
        let w_r =
            vec![E::Fr::from(2_u64), E::Fr::one(), E::Fr::one(), E::Fr::one()];
        let w_o = vec![E::Fr::one(), E::Fr::one(), E::Fr::one(), E::Fr::one()];
        let w_4 = vec![E::Fr::one(), E::Fr::one(), E::Fr::one(), E::Fr::one()];

        test_correct_permutation_poly(
            num_wire_mappings,
            perm,
            &domain,
            w_l,
            w_r,
            w_o,
            w_4,
        );
    }
    fn test_permutation_compute_sigmas<
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    >() {
        let mut perm: Permutation<E::Fr> = Permutation::new();

        let var_one = perm.new_variable();
        let var_two = perm.new_variable();
        let var_three = perm.new_variable();
        let var_four = perm.new_variable();

        let num_wire_mappings = 4;

        // Add four wire mappings
        perm.add_variables_to_map(var_one, var_one, var_two, var_four, 0);
        perm.add_variables_to_map(var_two, var_one, var_two, var_four, 1);
        perm.add_variables_to_map(var_three, var_three, var_one, var_four, 2);
        perm.add_variables_to_map(var_two, var_one, var_three, var_four, 3);

        /*
        Below is a sketch of the map created by adding the specific variables into the map
        var_one : {L0,R0, R1, O2, R3 }
        var_two : {O0, L1, O1, L3}
        var_three : {L2, R2, O3}
        var_four : {F0, F1, F2, F3}
        Left_Sigma : {0,1,2,3} -> {R0,O1,R2,O0}
        Right_Sigma : {0,1,2,3} -> {R1, O2, O3, L0}
        Out_Sigma : {0,1,2,3} -> {L1, L3, R3, L2}
        Fourth_Sigma : {0,1,2,3} -> {F1, F2, F3, F0}
        */
        let sigmas = perm.compute_sigma_permutations(num_wire_mappings);
        let left_sigma = &sigmas[0];
        let right_sigma = &sigmas[1];
        let out_sigma = &sigmas[2];
        let fourth_sigma = &sigmas[3];

        // Check the left sigma polynomial
        assert_eq!(left_sigma[0], WireData::Right(0));
        assert_eq!(left_sigma[1], WireData::Output(1));
        assert_eq!(left_sigma[2], WireData::Right(2));
        assert_eq!(left_sigma[3], WireData::Output(0));

        // Check the right sigma polynomial
        assert_eq!(right_sigma[0], WireData::Right(1));
        assert_eq!(right_sigma[1], WireData::Output(2));
        assert_eq!(right_sigma[2], WireData::Output(3));
        assert_eq!(right_sigma[3], WireData::Left(0));

        // Check the output sigma polynomial
        assert_eq!(out_sigma[0], WireData::Left(1));
        assert_eq!(out_sigma[1], WireData::Left(3));
        assert_eq!(out_sigma[2], WireData::Right(3));
        assert_eq!(out_sigma[3], WireData::Left(2));

        // Check the fourth sigma polynomial
        assert_eq!(fourth_sigma[0], WireData::Fourth(1));
        assert_eq!(fourth_sigma[1], WireData::Fourth(2));
        assert_eq!(fourth_sigma[2], WireData::Fourth(3));
        assert_eq!(fourth_sigma[3], WireData::Fourth(0));

        /*
        Check that the unique encodings of the sigma polynomials have been computed properly
        Left_Sigma : {R0,O1,R2,O0}
            When encoded using w, K1,K2,K3 we have {1 * K1, w * K2, w^2 * K1, 1 * K2}
        Right_Sigma : {R1, O2, O3, L0}
            When encoded using w, K1,K2,K3 we have {w * K1, w^2 * K2, w^3 * K2, 1}
        Out_Sigma : {L1, L3, R3, L2}
            When encoded using w, K1, K2,K3 we have {w, w^3 , w^3 * K1, w^2}
        Fourth_Sigma : {0,1,2,3} -> {F1, F2, F3, F0}
            When encoded using w, K1, K2,K3 we have {w * K3, w^2 * K3, w^3 * K3, 1 * K3}
        */
        let domain =
            GeneralEvaluationDomain::<E::Fr>::new(num_wire_mappings).unwrap();
        let w = domain.group_gen();
        let w_squared = w.pow(&[2, 0, 0, 0]);
        let w_cubed = w.pow(&[3, 0, 0, 0]);
        // check the left sigmas have been encoded properly
        let encoded_left_sigma =
            perm.compute_permutation_lagrange(left_sigma, &domain);
        assert_eq!(encoded_left_sigma[0], K1());
        assert_eq!(encoded_left_sigma[1], w * K2::<E::Fr>());
        assert_eq!(encoded_left_sigma[2], w_squared * K1::<E::Fr>());
        assert_eq!(encoded_left_sigma[3], E::Fr::one() * K2::<E::Fr>());

        // check the right sigmas have been encoded properly
        let encoded_right_sigma =
            perm.compute_permutation_lagrange(right_sigma, &domain);
        assert_eq!(encoded_right_sigma[0], w * K1::<E::Fr>());
        assert_eq!(encoded_right_sigma[1], w_squared * K2::<E::Fr>());
        assert_eq!(encoded_right_sigma[2], w_cubed * K2::<E::Fr>());
        assert_eq!(encoded_right_sigma[3], E::Fr::one());

        // check the output sigmas have been encoded properly
        let encoded_output_sigma =
            perm.compute_permutation_lagrange(out_sigma, &domain);
        assert_eq!(encoded_output_sigma[0], w);
        assert_eq!(encoded_output_sigma[1], w_cubed);
        assert_eq!(encoded_output_sigma[2], w_cubed * K1::<E::Fr>());
        assert_eq!(encoded_output_sigma[3], w_squared);

        // check the fourth sigmas have been encoded properly
        let encoded_fourth_sigma =
            perm.compute_permutation_lagrange(fourth_sigma, &domain);
        assert_eq!(encoded_fourth_sigma[0], w * K3::<E::Fr>());
        assert_eq!(encoded_fourth_sigma[1], w_squared * K3::<E::Fr>());
        assert_eq!(encoded_fourth_sigma[2], w_cubed * K3::<E::Fr>());
        assert_eq!(encoded_fourth_sigma[3], K3());
    }

    fn test_basic_slow_permutation_poly<
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    >() {
        let num_wire_mappings = 2;
        let mut perm = Permutation::new();
        let domain =
            GeneralEvaluationDomain::<E::Fr>::new(num_wire_mappings).unwrap();

        let var_one = perm.new_variable();
        let var_two = perm.new_variable();
        let var_three = perm.new_variable();
        let var_four = perm.new_variable();

        perm.add_variables_to_map(var_one, var_two, var_three, var_four, 0);
        perm.add_variables_to_map(var_three, var_two, var_one, var_four, 1);

        let w_l = vec![E::Fr::one(), E::Fr::from(3u64)];
        let w_r = vec![E::Fr::from(2u64), E::Fr::from(2u64)];
        let w_o = vec![E::Fr::from(3u64), E::Fr::one()];
        let w_4 = vec![E::Fr::one(), E::Fr::one()];

        test_correct_permutation_poly(
            num_wire_mappings,
            perm,
            &domain,
            w_l,
            w_r,
            w_o,
            w_4,
        );
    }

    // shifts the polynomials by one root of unity
    fn shift_poly_by_one<F: PrimeField>(z_coefficients: Vec<F>) -> Vec<F> {
        let mut shifted_z_coefficients = z_coefficients;
        shifted_z_coefficients.push(shifted_z_coefficients[0]);
        shifted_z_coefficients.remove(0);
        shifted_z_coefficients
    }

    fn test_correct_permutation_poly<F: PrimeField>(
        n: usize,
        mut perm: Permutation<F>,
        domain: &GeneralEvaluationDomain<F>,
        w_l: Vec<F>,
        w_r: Vec<F>,
        w_o: Vec<F>,
        w_4: Vec<F>,
    ) {
        // 0. Generate beta and gamma challenges
        //
        let beta = F::rand(&mut OsRng);
        let gamma = F::rand(&mut OsRng);
        assert_ne!(gamma, beta);

        //1. Compute the permutation polynomial using both methods
        //
        let (
            left_sigma_poly,
            right_sigma_poly,
            out_sigma_poly,
            fourth_sigma_poly,
        ) = perm.compute_sigma_polynomials(n, domain);
        let (z_vec, numerator_components, denominator_components) = perm
            .compute_slow_permutation_poly(
                domain,
                w_l.clone().into_iter(),
                w_r.clone().into_iter(),
                w_o.clone().into_iter(),
                w_4.clone().into_iter(),
                &beta,
                &gamma,
                (
                    &left_sigma_poly,
                    &right_sigma_poly,
                    &out_sigma_poly,
                    &fourth_sigma_poly,
                ),
            );

        let fast_z_vec = perm.compute_fast_permutation_poly(
            domain,
            &w_l,
            &w_r,
            &w_o,
            &w_4,
            beta,
            gamma,
            (
                &left_sigma_poly,
                &right_sigma_poly,
                &out_sigma_poly,
                &fourth_sigma_poly,
            ),
        );
        assert_eq!(fast_z_vec, z_vec);

        // 2. First we perform basic tests on the permutation vector
        //
        // Check that the vector has length `n` and that the first element is
        // `1`
        assert_eq!(z_vec.len(), n);
        assert_eq!(&z_vec[0], &F::one());
        //
        // Check that the \prod{f_i} / \prod{g_i} = 1
        // Where f_i and g_i are the numerator and denominator components in the
        // permutation polynomial
        let (mut a_0, mut b_0) = (F::one(), F::one());
        for n in numerator_components.iter() {
            a_0 *= n;
        }
        for n in denominator_components.iter() {
            b_0 *= n;
        }
        assert_eq!(a_0 * b_0.inverse().unwrap(), F::one());

        //3. Now we perform the two checks that need to be done on the
        // permutation polynomial (z)
        let z_poly =
            DensePolynomial::<F>::from_coefficients_vec(domain.ifft(&z_vec));
        //
        // Check that z(w^{n+1}) == z(1) == 1
        // This is the first check in the protocol
        assert_eq!(z_poly.evaluate(&F::one()), F::one());
        let n_plus_one = domain.elements().last().unwrap() * domain.group_gen();
        assert_eq!(z_poly.evaluate(&n_plus_one), F::one());
        //
        // Check that when z is unblinded, it has the correct degree
        assert_eq!(z_poly.degree(), n - 1);
        //
        // Check relationship between z(X) and z(Xw)
        // This is the second check in the protocol
        let roots: Vec<_> = domain.elements().collect();

        for i in 1..roots.len() {
            let current_root = roots[i];
            let next_root = current_root * domain.group_gen();

            let current_identity_perm_product = &numerator_components[i];
            assert_ne!(current_identity_perm_product, &F::zero());

            let current_copy_perm_product = &denominator_components[i];
            assert_ne!(current_copy_perm_product, &F::zero());

            assert_ne!(
                current_copy_perm_product,
                current_identity_perm_product
            );

            let z_eval = z_poly.evaluate(&current_root);
            assert_ne!(z_eval, F::zero());

            let z_eval_shifted = z_poly.evaluate(&next_root);
            assert_ne!(z_eval_shifted, F::zero());

            // Z(Xw) * copy_perm
            let lhs = z_eval_shifted * current_copy_perm_product;
            // Z(X) * iden_perm
            let rhs = z_eval * current_identity_perm_product;
            assert_eq!(
                lhs, rhs,
                "check failed at index: {}\'n lhs is : {:?} \n rhs is :{:?}",
                i, lhs, rhs
            );
        }

        // Test that the shifted polynomial is correct
        let shifted_z = shift_poly_by_one(fast_z_vec);
        let shifted_z_poly = DensePolynomial::<F>::from_coefficients_vec(
            domain.ifft(&shifted_z),
        );
        for element in domain.elements() {
            let z_eval = z_poly.evaluate(&(element * domain.group_gen()));
            let shifted_z_eval = shifted_z_poly.evaluate(&element);

            assert_eq!(z_eval, shifted_z_eval)
        }
    }

    // Test on Bls12-381
    batch_test!(
        [test_multizip_permutation_poly,
        test_permutation_format,
        test_permutation_compute_sigmas_only_left_wires,
        test_permutation_compute_sigmas,
        test_basic_slow_permutation_poly
        ],
        []
        => (
        Bls12_381,
        ark_ed_on_bls12_381::EdwardsParameters
        )
    );

    // Test on Bls12-377
    batch_test!(
        [test_multizip_permutation_poly,
        test_permutation_format,
        test_permutation_compute_sigmas_only_left_wires,
        test_permutation_compute_sigmas,
        test_basic_slow_permutation_poly
        ],
        []
        => (
        Bls12_377,
        ark_ed_on_bls12_377::EdwardsParameters
        )
    );
}
