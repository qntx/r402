//! XDR helpers for SEP-41 `transfer` transactions and auth-entry signing.

use std::str::FromStr;

use stellar_xdr::{
    AccountId, ContractEventBody, ContractEventType, ContractId, DecoratedSignature,
    DiagnosticEvent, Hash, HashIdPreimage, HashIdPreimageSorobanAuthorization, HostFunction,
    Int128Parts, InvokeContractArgs, InvokeHostFunctionOp, Limits, Memo, MuxedAccount, Operation,
    OperationBody, Preconditions, PublicKey, ReadXdr, ScAddress, ScBytes, ScMap, ScMapEntry,
    ScSymbol, ScVal, ScVec, SequenceNumber, Signature, SignatureHint, SorobanAddressCredentials,
    SorobanAuthorizationEntry, SorobanAuthorizedFunction, SorobanAuthorizedInvocation,
    SorobanCredentials, TimeBounds, TimePoint, Transaction, TransactionEnvelope, TransactionExt,
    TransactionV1Envelope, Uint256, WriteXdr,
};

use crate::TRANSFER_FUNCTION;
use crate::network_id;

/// Errors while encoding or decoding Stellar XDR.
#[derive(Debug, thiserror::Error)]
pub enum StellarXdrError {
    /// XDR codec failure.
    #[error("stellar xdr error: {0}")]
    Xdr(String),
    /// Transaction envelope shape is not a single invoke-host-function op.
    #[error("stellar transaction shape error: {0}")]
    Shape(String),
}

impl From<stellar_xdr::Error> for StellarXdrError {
    fn from(value: stellar_xdr::Error) -> Self {
        Self::Xdr(value.to_string())
    }
}

/// Decodes a base64 transaction envelope.
///
/// # Errors
///
/// Returns [`StellarXdrError::Xdr`] when the bytes are not a transaction envelope.
pub fn decode_transaction_envelope(xdr_b64: &str) -> Result<TransactionEnvelope, StellarXdrError> {
    Ok(TransactionEnvelope::from_xdr_base64(
        xdr_b64,
        Limits::none(),
    )?)
}

/// Encodes a transaction envelope as standard base64 XDR.
///
/// # Errors
///
/// Returns [`StellarXdrError::Xdr`] when serialization fails.
pub fn encode_transaction_envelope(
    envelope: &TransactionEnvelope,
) -> Result<String, StellarXdrError> {
    Ok(envelope.to_xdr_base64(Limits::none())?)
}

/// Returns the inner V1 transaction.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] for fee-bump or V0 envelopes.
pub fn inner_transaction(envelope: &TransactionEnvelope) -> Result<&Transaction, StellarXdrError> {
    match envelope {
        TransactionEnvelope::Tx(inner) => Ok(&inner.tx),
        TransactionEnvelope::TxV0(_) => Err(StellarXdrError::Shape(
            "transaction v0 envelopes are not supported".to_owned(),
        )),
        TransactionEnvelope::TxFeeBump(_) => Err(StellarXdrError::Shape(
            "fee-bump envelopes are not accepted as payment payloads".to_owned(),
        )),
    }
}

/// Returns the inner V1 transaction mutably.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] for fee-bump or V0 envelopes.
pub fn inner_transaction_mut(
    envelope: &mut TransactionEnvelope,
) -> Result<&mut Transaction, StellarXdrError> {
    match envelope {
        TransactionEnvelope::Tx(inner) => Ok(&mut inner.tx),
        TransactionEnvelope::TxV0(_) => Err(StellarXdrError::Shape(
            "transaction v0 envelopes are not supported".to_owned(),
        )),
        TransactionEnvelope::TxFeeBump(_) => Err(StellarXdrError::Shape(
            "fee-bump envelopes are not accepted as payment payloads".to_owned(),
        )),
    }
}

/// Display form of a muxed account (G or M).
#[must_use]
pub fn muxed_account_to_string(account: &MuxedAccount) -> String {
    account.to_string()
}

/// Underlying ed25519 payload of a G or muxed-G account.
#[must_use]
pub const fn muxed_account_ed25519(account: &MuxedAccount) -> [u8; 32] {
    match account {
        MuxedAccount::Ed25519(Uint256(bytes)) => *bytes,
        MuxedAccount::MuxedEd25519(med) => med.ed25519.0,
    }
}

/// Parses a G/M address as a muxed account.
///
/// # Errors
///
/// Returns [`StellarXdrError::Xdr`] when the string is not a G or M address.
pub fn muxed_account_from_str(address: &str) -> Result<MuxedAccount, StellarXdrError> {
    MuxedAccount::from_str(address).map_err(|e| StellarXdrError::Xdr(e.to_string()))
}

/// Display form of an `ScAddress` (G, C, or M).
#[must_use]
pub fn sc_address_to_string(address: &ScAddress) -> String {
    address.to_string()
}

/// Parses a G/C/M address as `ScAddress`.
///
/// # Errors
///
/// Returns [`StellarXdrError::Xdr`] when the string is not a Stellar address.
pub fn sc_address_from_str(address: &str) -> Result<ScAddress, StellarXdrError> {
    ScAddress::from_str(address).map_err(|e| StellarXdrError::Xdr(e.to_string()))
}

/// Converts `ScVal` to a Stellar address string.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] when the value is not an address.
pub fn sc_val_to_address(value: &ScVal) -> Result<String, StellarXdrError> {
    match value {
        ScVal::Address(address) => Ok(sc_address_to_string(address)),
        _ => Err(StellarXdrError::Shape(
            "expected scvAddress transfer argument".to_owned(),
        )),
    }
}

/// Converts `ScVal` to `i128`.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] when the value is not `scvI128`.
pub fn sc_val_to_i128(value: &ScVal) -> Result<i128, StellarXdrError> {
    match value {
        ScVal::I128(parts) => Ok(i128_from_parts(parts.hi, parts.lo)),
        _ => Err(StellarXdrError::Shape(
            "expected scvI128 transfer amount".to_owned(),
        )),
    }
}

/// Packs a signed 128-bit integer into Stellar `Int128Parts`.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "XDR Int128Parts is the two's-complement split of i128"
)]
pub const fn i128_to_parts(value: i128) -> Int128Parts {
    Int128Parts {
        hi: (value >> 64) as i64,
        lo: value as u64,
    }
}

/// Unpacks Stellar `Int128Parts` into `i128`.
#[must_use]
pub const fn i128_from_parts(hi: i64, lo: u64) -> i128 {
    ((hi as i128) << 64) | (lo as i128)
}

/// Builds `ScVal::Address` from a G/C/M string.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when the address is invalid.
pub fn sc_val_address(address: &str) -> Result<ScVal, StellarXdrError> {
    Ok(ScVal::Address(sc_address_from_str(address)?))
}

/// Builds `ScVal::I128` from an amount.
#[must_use]
pub const fn sc_val_i128(amount: i128) -> ScVal {
    ScVal::I128(i128_to_parts(amount))
}

/// Builds the SEP-41 `transfer(from, to, amount)` host function.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when addresses or XDR conversions fail.
pub fn transfer_host_function(
    asset: &str,
    from: &str,
    to: &str,
    amount: i128,
) -> Result<HostFunction, StellarXdrError> {
    let function_name = ScSymbol(
        TRANSFER_FUNCTION
            .try_into()
            .map_err(|e: stellar_xdr::Error| StellarXdrError::Xdr(e.to_string()))?,
    );
    let args = vec![
        sc_val_address(from)?,
        sc_val_address(to)?,
        sc_val_i128(amount),
    ];
    Ok(HostFunction::InvokeContract(InvokeContractArgs {
        contract_address: sc_address_from_str(asset)?,
        function_name,
        args: args.try_into()?,
    }))
}

/// Builds an unsigned V1 envelope invoking `transfer`.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when the transaction cannot be encoded.
#[allow(
    clippy::too_many_arguments,
    reason = "transaction fields are independent inputs"
)]
pub fn build_transfer_envelope(
    source: &str,
    asset: &str,
    from: &str,
    to: &str,
    amount: i128,
    seq_num: i64,
    fee: u32,
    auth: Vec<SorobanAuthorizationEntry>,
    ext: TransactionExt,
    timeout_seconds: u64,
    now_unix: u64,
) -> Result<TransactionEnvelope, StellarXdrError> {
    let host_function = transfer_host_function(asset, from, to, amount)?;
    let operation = Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function,
            auth: auth.try_into()?,
        }),
    };
    let max_time = now_unix.saturating_add(timeout_seconds);
    let tx = Transaction {
        source_account: muxed_account_from_str(source)?,
        fee,
        seq_num: SequenceNumber(seq_num),
        cond: Preconditions::Time(TimeBounds {
            min_time: TimePoint(0),
            max_time: TimePoint(max_time),
        }),
        memo: Memo::None,
        operations: vec![operation].try_into()?,
        ext,
    };
    Ok(TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: Vec::new().try_into()?,
    }))
}

/// Extracts the invoke-host-function operation.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] when the transaction is not a single
/// invoke-host-function operation.
pub fn invoke_host_function_op(tx: &Transaction) -> Result<&InvokeHostFunctionOp, StellarXdrError> {
    if tx.operations.len() != 1 {
        return Err(StellarXdrError::Shape(format!(
            "expected exactly one operation, got {}",
            tx.operations.len()
        )));
    }
    let operation = tx
        .operations
        .first()
        .ok_or_else(|| StellarXdrError::Shape("missing operation".to_owned()))?;
    match &operation.body {
        OperationBody::InvokeHostFunction(op) => Ok(op),
        _ => Err(StellarXdrError::Shape(
            "expected InvokeHostFunction operation".to_owned(),
        )),
    }
}

/// Extracts the invoke-host-function operation mutably.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] when the transaction is not a single
/// invoke-host-function operation.
pub fn invoke_host_function_op_mut(
    tx: &mut Transaction,
) -> Result<&mut InvokeHostFunctionOp, StellarXdrError> {
    if tx.operations.len() != 1 {
        return Err(StellarXdrError::Shape(format!(
            "expected exactly one operation, got {}",
            tx.operations.len()
        )));
    }
    let operation = tx
        .operations
        .iter_mut()
        .next()
        .ok_or_else(|| StellarXdrError::Shape("missing operation".to_owned()))?;
    match &mut operation.body {
        OperationBody::InvokeHostFunction(op) => Ok(op),
        _ => Err(StellarXdrError::Shape(
            "expected InvokeHostFunction operation".to_owned(),
        )),
    }
}

/// Parsed SEP-41 transfer invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferInvocation {
    /// Token contract address.
    pub asset: String,
    /// `from` argument.
    pub from: String,
    /// `to` argument.
    pub to: String,
    /// Transfer amount.
    pub amount: i128,
}

/// Parses a `transfer(from, to, amount)` host function.
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] when the function is not a 3-arg transfer.
pub fn parse_transfer_invocation(
    host_function: &HostFunction,
) -> Result<TransferInvocation, StellarXdrError> {
    let HostFunction::InvokeContract(args) = host_function else {
        return Err(StellarXdrError::Shape(
            "expected hostFunctionTypeInvokeContract".to_owned(),
        ));
    };
    let name = symbol_to_string(&args.function_name);
    if name != TRANSFER_FUNCTION || args.args.len() != 3 {
        return Err(StellarXdrError::Shape(
            "expected transfer with exactly 3 arguments".to_owned(),
        ));
    }
    let from = sc_val_to_address(
        args.args
            .first()
            .ok_or_else(|| StellarXdrError::Shape("missing transfer from argument".to_owned()))?,
    )?;
    let to = sc_val_to_address(
        args.args
            .get(1)
            .ok_or_else(|| StellarXdrError::Shape("missing transfer to argument".to_owned()))?,
    )?;
    let amount =
        sc_val_to_i128(args.args.get(2).ok_or_else(|| {
            StellarXdrError::Shape("missing transfer amount argument".to_owned())
        })?)?;
    Ok(TransferInvocation {
        asset: sc_address_to_string(&args.contract_address),
        from,
        to,
        amount,
    })
}

fn symbol_to_string(symbol: &ScSymbol) -> String {
    String::from_utf8_lossy(symbol.as_ref()).into_owned()
}

/// Returns `true` when the auth-entry signature is present (not `scvVoid`).
#[must_use]
pub const fn auth_entry_is_signed(entry: &SorobanAuthorizationEntry) -> bool {
    match &entry.credentials {
        SorobanCredentials::Address(creds) | SorobanCredentials::AddressV2(creds) => {
            !matches!(creds.signature, ScVal::Void)
        }
        SorobanCredentials::SourceAccount | SorobanCredentials::AddressWithDelegates(_) => false,
    }
}

/// Address of an address-credential auth entry.
#[must_use]
pub fn auth_entry_address(entry: &SorobanAuthorizationEntry) -> Option<String> {
    match &entry.credentials {
        SorobanCredentials::Address(creds) | SorobanCredentials::AddressV2(creds) => {
            Some(sc_address_to_string(&creds.address))
        }
        SorobanCredentials::SourceAccount | SorobanCredentials::AddressWithDelegates(_) => None,
    }
}

/// Auth-entry signature status after dropping source-account credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthEntrySignatureStatus {
    /// Addresses that already signed.
    pub already_signed: Vec<String>,
    /// Addresses still pending a signature.
    pub pending_signature: Vec<String>,
}

/// Categorizes address-credential auth entries by signature presence.
#[must_use]
pub fn gather_auth_entry_signature_status(
    auth: &[SorobanAuthorizationEntry],
) -> AuthEntrySignatureStatus {
    let mut already_signed = Vec::new();
    let mut pending_signature = Vec::new();
    for entry in auth {
        if matches!(entry.credentials, SorobanCredentials::SourceAccount) {
            continue;
        }
        let Some(address) = auth_entry_address(entry) else {
            continue;
        };
        if auth_entry_is_signed(entry) {
            if !already_signed.contains(&address) {
                already_signed.push(address);
            }
        } else if !pending_signature.contains(&address) {
            pending_signature.push(address);
        }
    }
    AuthEntrySignatureStatus {
        already_signed,
        pending_signature,
    }
}

/// Parsed transfer event from a diagnostic contract event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEvent {
    /// Token contract that emitted the event.
    pub asset: String,
    /// Debited address.
    pub from: String,
    /// Credited address.
    pub to: String,
    /// Transferred amount.
    pub amount: i128,
}

/// Parses a CAP-46 `transfer` diagnostic event.
///
/// Returns `Ok(None)` for non-contract events. Contract events that are not
/// a well-formed transfer fail with [`StellarXdrError::Shape`].
///
/// # Errors
///
/// Returns [`StellarXdrError::Shape`] when a contract event is not a transfer.
pub fn parse_transfer_event(
    event: &DiagnosticEvent,
) -> Result<Option<TransferEvent>, StellarXdrError> {
    if event.event.type_ != ContractEventType::Contract {
        return Ok(None);
    }
    let ContractEventBody::V0(body) = &event.event.body;
    let topic0 = body.topics.first().ok_or_else(|| {
        StellarXdrError::Shape("contract event missing transfer topic".to_owned())
    })?;
    let ScVal::Symbol(symbol) = topic0 else {
        return Err(StellarXdrError::Shape(
            "contract event topic is not a symbol".to_owned(),
        ));
    };
    if symbol_to_string(symbol) != "transfer" {
        return Err(StellarXdrError::Shape(
            "contract event is not a transfer".to_owned(),
        ));
    }
    if body.topics.len() < 3 {
        return Err(StellarXdrError::Shape(
            "transfer event has fewer than 3 topics".to_owned(),
        ));
    }
    let contract_id =
        event.event.contract_id.as_ref().ok_or_else(|| {
            StellarXdrError::Shape("transfer event missing contract id".to_owned())
        })?;
    let from =
        sc_val_to_address(body.topics.get(1).ok_or_else(|| {
            StellarXdrError::Shape("transfer event missing from topic".to_owned())
        })?)?;
    let to =
        sc_val_to_address(body.topics.get(2).ok_or_else(|| {
            StellarXdrError::Shape("transfer event missing to topic".to_owned())
        })?)?;
    Ok(Some(TransferEvent {
        asset: ContractId(contract_id.0.clone()).to_string(),
        from,
        to,
        amount: sc_val_to_i128(&body.data)?,
    }))
}

/// Hashes a Soroban authorization preimage.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when the preimage cannot be encoded.
pub fn auth_entry_preimage_hash(
    passphrase: &str,
    nonce: i64,
    signature_expiration_ledger: u32,
    invocation: &SorobanAuthorizedInvocation,
) -> Result<[u8; 32], StellarXdrError> {
    use sha2::{Digest, Sha256};
    let preimage = HashIdPreimage::SorobanAuthorization(HashIdPreimageSorobanAuthorization {
        network_id: Hash(network_id(passphrase)),
        nonce,
        signature_expiration_ledger,
        invocation: invocation.clone(),
    });
    let bytes = preimage.to_xdr(Limits::none())?;
    Ok(Sha256::digest(bytes).into())
}

/// Encodes a G-account `__check_auth` signature as `Vec<AccountEd25519Signature>`.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when the map cannot be encoded.
pub fn account_ed25519_signature_val(
    public_key: &[u8; 32],
    signature: &[u8; 64],
) -> Result<ScVal, StellarXdrError> {
    let public_key_sym = sc_symbol("public_key")?;
    let signature_sym = sc_symbol("signature")?;
    let entry = ScVal::Map(Some(ScMap(
        vec![
            ScMapEntry {
                key: ScVal::Symbol(public_key_sym),
                val: ScVal::Bytes(ScBytes(public_key.to_vec().try_into()?)),
            },
            ScMapEntry {
                key: ScVal::Symbol(signature_sym),
                val: ScVal::Bytes(ScBytes(signature.to_vec().try_into()?)),
            },
        ]
        .try_into()?,
    )));
    Ok(ScVal::Vec(Some(ScVec(vec![entry].try_into()?))))
}

fn sc_symbol(name: &str) -> Result<ScSymbol, StellarXdrError> {
    Ok(ScSymbol(name.try_into()?))
}

/// Signs address-credential auth entries that match `address`.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when hashing or encoding fails.
pub fn sign_auth_entries_for_address(
    auth: &mut [SorobanAuthorizationEntry],
    address: &str,
    passphrase: &str,
    expiration_ledger: u32,
    public_key: &[u8; 32],
    sign: impl Fn(&[u8; 32]) -> [u8; 64],
) -> Result<usize, StellarXdrError> {
    let mut signed = 0usize;
    for entry in auth {
        let SorobanCredentials::Address(creds) = &mut entry.credentials else {
            continue;
        };
        if sc_address_to_string(&creds.address) != address {
            continue;
        }
        creds.signature_expiration_ledger = expiration_ledger;
        let hash = auth_entry_preimage_hash(
            passphrase,
            creds.nonce,
            creds.signature_expiration_ledger,
            &entry.root_invocation,
        )?;
        let signature = sign(&hash);
        creds.signature = account_ed25519_signature_val(public_key, &signature)?;
        signed = signed.saturating_add(1);
    }
    Ok(signed)
}

/// Builds a decorated transaction signature.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when the 64-byte signature cannot be encoded.
pub fn decorated_signature(
    public_key: &[u8; 32],
    signature: &[u8; 64],
) -> Result<DecoratedSignature, StellarXdrError> {
    let hint = public_key
        .get(28..)
        .and_then(|tail| tail.try_into().ok())
        .ok_or_else(|| StellarXdrError::Shape("public key too short for hint".to_owned()))?;
    Ok(DecoratedSignature {
        hint: SignatureHint(hint),
        signature: Signature(signature.to_vec().try_into()?),
    })
}

/// Signs a V1 transaction envelope with one Ed25519 key.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when hashing or encoding fails.
pub fn sign_transaction_envelope(
    envelope: &mut TransactionEnvelope,
    passphrase: &str,
    public_key: &[u8; 32],
    sign: impl Fn(&[u8; 32]) -> [u8; 64],
) -> Result<(), StellarXdrError> {
    let hash = envelope.hash(network_id(passphrase))?;
    let signature = sign(&hash);
    let decorated = decorated_signature(public_key, &signature)?;
    match envelope {
        TransactionEnvelope::Tx(inner) => {
            inner.signatures = vec![decorated].try_into()?;
        }
        TransactionEnvelope::TxFeeBump(inner) => {
            inner.signatures = vec![decorated].try_into()?;
        }
        TransactionEnvelope::TxV0(inner) => {
            inner.signatures = vec![decorated].try_into()?;
        }
    }
    Ok(())
}

/// Builds an empty address-credential auth entry for tests.
#[must_use]
pub fn unsigned_address_auth(
    address: &str,
    nonce: i64,
    expiration: u32,
    host_function: &HostFunction,
) -> Option<SorobanAuthorizationEntry> {
    let HostFunction::InvokeContract(args) = host_function else {
        return None;
    };
    Some(SorobanAuthorizationEntry {
        credentials: SorobanCredentials::Address(SorobanAddressCredentials {
            address: sc_address_from_str(address).ok()?,
            nonce,
            signature_expiration_ledger: expiration,
            signature: ScVal::Void,
        }),
        root_invocation: SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(args.clone()),
            sub_invocations: Vec::new().try_into().ok()?,
        },
    })
}

/// Ed25519 public key bytes from a G-address.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when the address is not a G-account.
pub fn account_public_key_bytes(address: &str) -> Result<[u8; 32], StellarXdrError> {
    let account = AccountId::from_str(address).map_err(|e| StellarXdrError::Xdr(e.to_string()))?;
    match account.0 {
        PublicKey::PublicKeyTypeEd25519(Uint256(bytes)) => Ok(bytes),
    }
}
