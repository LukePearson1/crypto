// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Copyright (c) ZK-INFRA. All rights reserved.

//! Range Gate

use crate::constraint_system::{StandardComposer, Variable, WireData};
use ark_ec::{PairingEngine, TEModelParameters};
use ark_ff::{BigInteger, PrimeField};
use num_traits::{One, Zero};

impl<E, P> StandardComposer<E, P>
where
    E: PairingEngine,
    P: TEModelParameters<BaseField = E::Fr>,
{
    /// Adds a range-constraint gate that checks and constrains a
    /// [`Variable`] to be inside of the range \[0,num_bits\].
    ///
    /// This function adds `num_bits/4` gates to the circuit description in
    /// order to add the range constraint.
    ///
    ///# Panics
    /// This function will panic if the num_bits specified is not even, ie.
    /// `num_bits % 2 != 0`.
    pub fn range_gate(&mut self, witness: Variable, num_bits: usize) {
        // Adds `variable` into the appropriate witness position
        // based on the accumulator number a_i
        let add_wire = |composer: &mut StandardComposer<E, P>,
                        i: usize,
                        variable: Variable| {
            // Since four quads can fit into one gate, the gate index does
            // not change for every four wires
            let gate_index = composer.circuit_size() + (i / 4);

            let wire_data = match i % 4 {
                0 => {
                    composer.w_4.push(variable);
                    WireData::Fourth(gate_index)
                }
                1 => {
                    composer.w_o.push(variable);
                    WireData::Output(gate_index)
                }
                2 => {
                    composer.w_r.push(variable);
                    WireData::Right(gate_index)
                }
                3 => {
                    composer.w_l.push(variable);
                    WireData::Left(gate_index)
                }
                _ => unreachable!(),
            };
            composer.perm.add_variable_to_map(variable, wire_data);
        };

        // Note: A quad is a quaternary digit
        //
        // Number of bits should be even, this means that user must pad the
        // number of bits external.
        assert!(num_bits % 2 == 0);

        // Convert witness to bit representation and reverse
        let bits = self.variables[&witness].into_repr().to_bits_le();

        // For a width-4 program, one gate will contain 4 accumulators
        // Each accumulator proves that a single quad is a base-4 digit.
        // Since there is 1-1 mapping between accumulators and quads
        // and quads contain 2 bits, one gate accumulates 8 bits.
        // We can therefore work out the number of gates needed;
        let mut num_gates = num_bits >> 3;

        // The number of bits may be divisible by 2 but not by 8.
        // Example: If we wanted to prove a number was within the range [0,2^10
        // -1 ] We would need 10 bits. When dividing by 10 by 8, we will
        // get 1 as the number of gates, when in fact we need 2 gates In
        // general, we will need to add an extra gate, if the number of bits is
        // not divisible by 8
        if num_bits % 8 != 0 {
            num_gates += 1;
        }

        // Since each gate holds 4 quads, the number of quads that will be
        // needed to prove that the witness is within a specific range can be
        // computed from the number of gates
        let num_quads = num_gates * 4;

        // There are now two things to note in terms of padding:
        // 1. (a_{i+1}, a_i) proves that {q_i+1} is a quaternary digit.
        // In order to prove that the first digit is a quad, we need to add a
        // zero accumulator (genesis quad) 2. We need the last gate to
        // contain 1 quad, so the range gate equation is not used on the last
        // gate. This is needed because the range gate equation looks at
        // the fourth for the next gate, which is not guaranteed to pass.
        // We therefore prepend quads until we have 1 quad in the last gate.
        // This will at most add one extra gate.
        //
        // There are two cases to consider:
        // Case 1: If the number of bits used is divisible by 8, then it is also
        // divisible by 4. This means that we can find out how many
        // gates are needed by dividing the number of bits by 8 However,
        // since we will always need a genesis quad, it will mean that we will
        // need an another gate to hold the extra quad Example: Take 32
        // bits. We compute the number of gates to be 32/8 = 4 full gates, we
        // then add 1 because we need the genesis accumulator
        // In this case, we only pad by one quad, which is the genesis quad.
        // Moreover, the genesis quad is the quad that has added the extra gate.
        //
        // Case 2: When the number of bits is not divisible by 8
        // Since the number is not divisible by 4, as in case 1, when we add the
        // genesis quad, we will have more than 1 quad on the last row
        // In this case, the genesis quad, did not add an extra gate. What will
        // add the extra gate, is the padding. We must apply padding in
        // order to ensure the last row has only one quad in on the fourth wire
        // In this case, it is the padding which will add an extra number of
        // gates Example: 34 bits requires 17 quads. We add one for the
        // zeroed out accumulator. To make 18 quads. We can fit all of these
        // quads in 5 gates. 18 % 4 = 2 so on the last row, we will have
        // two quads, which is bad. We must pad the beginning in order
        // to get one quad on the last row We can work out how much we
        // need to pad by the following equation (18+X) % 4 = 1
        // X is 3 , so we pad 3 extra zeroes
        // We now have 21 quads in the system now and 21 / 4 = 5 remainder 1, so
        // we will need 5 full gates and extra gate with 1 quad.
        let pad = 1 + (((num_quads << 1) - num_bits) >> 1);

        // The last observation; we will always use 1 more gate than the number
        // of gates calculated Either due to the genesis quad, or the
        // padding used to ensure we have 1 quad on the last gate
        let used_gates = num_gates + 1;

        // We collect the set of accumulators to return back to the user
        // and keep a running count of the current accumulator
        let mut accumulators: Vec<Variable> = Vec::new();
        let mut accumulator = E::Fr::zero();
        let four = E::Fr::from(4u64);

        // First we pad our gates by the necessary amount
        for i in 0..pad {
            add_wire(self, i, self.zero_var);
        }

        for i in pad..=num_quads {
            // Convert each pair of bits to quads
            let bit_index = (num_quads - i) << 1;
            let q_0 = bits[bit_index] as u64;
            let q_1 = bits[bit_index + 1] as u64;
            let quad = q_0 + (2 * q_1);

            // Compute the next accumulator term
            accumulator = four * accumulator;
            accumulator += E::Fr::from(quad);

            let accumulator_var = self.add_input(accumulator);
            accumulators.push(accumulator_var);

            add_wire(self, i, accumulator_var);
        }

        // Set the selector polynomials for all of the gates we used
        let zeros = vec![E::Fr::zero(); used_gates];
        let ones = vec![E::Fr::one(); used_gates];

        self.q_m.extend(zeros.iter());
        self.q_l.extend(zeros.iter());
        self.q_r.extend(zeros.iter());
        self.q_o.extend(zeros.iter());
        self.q_c.extend(zeros.iter());
        self.q_arith.extend(zeros.iter());
        self.q_4.extend(zeros.iter());
        self.q_fixed_group_add.extend(zeros.iter());
        self.q_variable_group_add.extend(zeros.iter());
        self.q_range.extend(ones.iter());
        self.q_logic.extend(zeros.iter());
        self.n += used_gates;

        // As mentioned above, we must switch off the range constraint for the
        // last gate Remember; it will contain one quad in the fourth
        // wire, which will be used in the gate before it
        // Furthermore, we set the left, right and output wires to zero
        *self.q_range.last_mut().unwrap() = E::Fr::zero();
        self.w_l.push(self.zero_var);
        self.w_r.push(self.zero_var);
        self.w_o.push(self.zero_var);

        // Lastly, we must link the last accumulator value to the initial
        // witness This last constraint will pass as long as
        // - The witness is within the number of bits initially specified
        let last_accumulator = accumulators.len() - 1;
        self.assert_equal(accumulators[last_accumulator], witness);
        accumulators[last_accumulator] = witness;
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{batch_test, constraint_system::helper::*};
    use ark_bls12_377::Bls12_377;
    use ark_bls12_381::Bls12_381;

    fn test_range_constraint<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        // Should fail as the number is not 32 bits
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let witness = composer
                    .add_input(E::Fr::from((u32::max_value() as u64) + 1));
                composer.range_gate(witness, 32);
            },
            200,
        );
        assert!(res.is_err());

        // Should fail as number is greater than 32 bits
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let witness = composer.add_input(E::Fr::from(u64::max_value()));
                composer.range_gate(witness, 32);
            },
            200,
        );
        assert!(res.is_err());

        // Should pass as the number is within 34 bits
        let res = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let witness = composer.add_input(E::Fr::from(2u64.pow(34) - 1));
                composer.range_gate(witness, 34);
            },
            200,
        );
        assert!(res.is_ok());
    }

    fn test_odd_bit_range<E, P>()
    where
        E: PairingEngine,
        P: TEModelParameters<BaseField = E::Fr>,
    {
        // Should fail as the number we we need a even number of bits
        let _ok = gadget_tester(
            |composer: &mut StandardComposer<E, P>| {
                let witness =
                    composer.add_input(E::Fr::from(u32::max_value() as u64));
                composer.range_gate(witness, 33);
            },
            200,
        );
    }

    // Test on Bls12-381
    batch_test!(
        [test_range_constraint],
        [test_odd_bit_range]
        => (
            Bls12_381,
            ark_ed_on_bls12_381::EdwardsParameters
        )
    );

    // Test on Bls12-377
    batch_test!(
        [test_range_constraint],
        [test_odd_bit_range]
        => (
            Bls12_377,
            ark_ed_on_bls12_377::EdwardsParameters
        )
    );
}
