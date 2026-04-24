//! SPL Memo instruction enforcement.
//!
//! Per x402 v2 §SVM exact scheme, when the seller declares
//! [`SupportedPaymentKindExtra::memo`](super::super::types::SupportedPaymentKindExtra::memo),
//! the client-submitted transaction MUST include **exactly one** SPL Memo
//! instruction whose data bytes equal the declared memo string.

use solana_message::compiled_instruction::CompiledInstruction;
use solana_pubkey::Pubkey;

use crate::exact::{SPL_MEMO_PROGRAM, SolanaExactError};

/// Verifies the memo instruction requirement against a transaction's
/// compiled instructions and static account keys.
///
/// # Behavior
///
/// - `expected = None` → no requirement; any number of memo instructions
///   is permitted.
/// - `expected = Some(memo)` → the transaction MUST contain exactly one
///   memo instruction whose data equals `memo.as_bytes()`.
///
/// # Errors
///
/// - [`SolanaExactError::MemoInstructionCountInvalid`] when the memo count
///   is ≠ 1 (and a memo is required).
/// - [`SolanaExactError::MemoMismatch`] when the memo data differs.
pub(crate) fn verify_memo(
    instructions: &[CompiledInstruction],
    static_keys: &[Pubkey],
    expected: Option<&str>,
) -> Result<(), SolanaExactError> {
    let Some(expected) = expected else {
        return Ok(());
    };

    let memo_instructions: Vec<&CompiledInstruction> = instructions
        .iter()
        .filter(|ix| {
            static_keys
                .get(usize::from(ix.program_id_index))
                .is_some_and(|pk| *pk == SPL_MEMO_PROGRAM)
        })
        .collect();

    let count = memo_instructions.len();
    if count != 1 {
        return Err(SolanaExactError::MemoInstructionCountInvalid { count });
    }

    let data = &memo_instructions[0].data;
    if data.as_slice() != expected.as_bytes() {
        return Err(SolanaExactError::MemoMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ix(program_id_index: u8, data: &[u8]) -> CompiledInstruction {
        CompiledInstruction {
            program_id_index,
            accounts: Vec::new(),
            data: data.to_vec(),
        }
    }

    fn keys() -> Vec<Pubkey> {
        let payer = Pubkey::new_from_array([1u8; 32]);
        vec![payer, SPL_MEMO_PROGRAM]
    }

    #[test]
    fn no_requirement_accepts_any() {
        assert!(verify_memo(&[], &keys(), None).is_ok());
        let ix = make_ix(1, b"arbitrary");
        assert!(verify_memo(&[ix], &keys(), None).is_ok());
    }

    #[test]
    fn missing_memo_rejected() {
        assert!(matches!(
            verify_memo(&[], &keys(), Some("order-123")),
            Err(SolanaExactError::MemoInstructionCountInvalid { count: 0 }),
        ));
    }

    #[test]
    fn mismatched_memo_rejected() {
        let ix = make_ix(1, b"wrong");
        assert!(matches!(
            verify_memo(&[ix], &keys(), Some("order-123")),
            Err(SolanaExactError::MemoMismatch),
        ));
    }

    #[test]
    fn matching_memo_accepted() {
        let ix = make_ix(1, b"order-123");
        assert!(verify_memo(&[ix], &keys(), Some("order-123")).is_ok());
    }

    #[test]
    fn multiple_memos_rejected() {
        let ix = make_ix(1, b"order-123");
        let ixs = vec![ix.clone(), ix];
        assert!(matches!(
            verify_memo(&ixs, &keys(), Some("order-123")),
            Err(SolanaExactError::MemoInstructionCountInvalid { count: 2 }),
        ));
    }
}
