//! The mesa account, and how the protocol hashes it.
//!
//! Mesa (protocol transaction version 3) widened the zkApp application state from **8**
//! field elements to **32**. `mina-tree`'s [`ZkAppAccount`] is `[Fp; 8]`, so it cannot
//! represent -- let alone hash -- a mesa account, and every mesa ledger root and account
//! inclusion proof is out of reach with it.
//!
//! Widening `[Fp; 8]` inside mina-tree is the wrong fix: `app_state`'s arity is
//! load-bearing in ~140 places there, 60 of them in the Pickles circuits, whose mesa
//! semantics we do not know and do not need. So this mirrors the two account types with
//! the wider state, and hashes them with mina-tree's *public* primitives -- the same
//! `ToInputs` packing, the same Poseidon parameters. No fork.
//!
//! The account-level packing is unchanged from V2: [`Account::to_inputs`] folds the whole
//! zkApp into a single field via `zkapp.hash()`, so the wider state is confined to
//! [`MesaZkAppAccount::to_inputs`]. That is the entire protocol delta.

use mina_tree::{
    scan_state::currency::{Balance, Magnitude, Nonce, Slot},
    AppendToInputs, MyCow, Permissions, ReceiptChainHash, Timing, TimingAsRecord, ToInputs,
    TokenId, TokenSymbol, VerificationKeyWire, VotingFor, ZkAppUri,
};
use mina_curves::pasta::Fp;
use mina_signer::CompressedPubKey;
use once_cell::sync::Lazy;
use poseidon::hash::{
    params::{MINA_ACCOUNT, MINA_ZKAPP_ACCOUNT},
    Inputs,
};

/// Mesa's zkApp application state width. V2 is 8.
pub const MESA_ZKAPP_STATE_SIZE: usize = 32;

/// V2's (devnet / mainnet-hardfork) zkApp application state width.
pub const V2_ZKAPP_STATE_SIZE: usize = 8;

/// mesa-mut's transaction version.
///
/// This matters far more than it looks. `Account::empty()` takes its permissions from
/// `Permissions::user_default()`, whose `set_verification_key` carries
/// `Txn_version.current` -- so the transaction version is hashed into the **empty
/// account**, and therefore into every empty subtree, and therefore into every ledger
/// root. mina-tree compiles in `TXN_VERSION_CURRENT = 3` (Berkeley). mesa-mut is **4**, so
/// borrowing mina-tree's `user_default()` silently corrupts every mesa root.
///
/// Recovered from the protocol itself: the mesa-mut daemon hashes the empty ledger to
/// `jwkaDMeS...`, and only (width 32, txn_version 4) reproduces it. See
/// `tests/mesa_solve_empty.rs`.
pub const MESA_TXN_VERSION: u32 = 4;

/// Berkeley / V2's transaction version -- what mina-tree compiles in.
pub const V2_TXN_VERSION: u32 = 3;

/// `Permissions::user_default()` for a given transaction version. mina-tree's hardcodes
/// its own compiled-in version, which is not mesa's.
pub fn user_default_permissions(txn_version: u32) -> Permissions<mina_tree::AuthRequired> {
    let mut permissions = Permissions::user_default();

    permissions.set_verification_key.txn_version =
        mina_tree::scan_state::currency::TxnVersion::from_u32(txn_version);

    permissions
}

/// A zkApp account with an `N`-wide application state. `N = 8` is V2; `N = 32` is mesa.
///
/// The width is a parameter rather than a constant so the *same* code can be run against a
/// V2 ledger, whose root the protocol also published -- that is how this implementation is
/// held honest (see `tests/berkeley_v2_control.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZkAppAccountOf<const N: usize> {
    pub app_state: [Fp; N],
    pub verification_key: Option<VerificationKeyWire>,
    pub zkapp_version: u32,
    pub action_state: [Fp; 5],
    pub last_action_slot: Slot,
    pub proved_state: bool,
    pub zkapp_uri: ZkAppUri,
}

/// The mesa zkApp account: 32 field elements of application state.
pub type MesaZkAppAccount = ZkAppAccountOf<MESA_ZKAPP_STATE_SIZE>;

/// The V2 zkApp account, for the control test.
pub type V2ZkAppAccount = ZkAppAccountOf<V2_ZKAPP_STATE_SIZE>;

impl<const N: usize> Default for ZkAppAccountOf<N> {
    fn default() -> Self {
        Self {
            app_state: [Fp::from(0); N],
            verification_key: None,
            zkapp_version: 0,
            // the empty action state, as in mina-tree's `ZkAppAccount::default`
            action_state: [mina_tree::ZkAppAccount::empty_action_state(); 5],
            last_action_slot: Slot::zero(),
            proved_state: false,
            zkapp_uri: ZkAppUri::new(),
        }
    }
}

/// Field order is mina-tree's `impl ToInputs for ZkAppAccount`, verbatim -- only the
/// `app_state` loop is wider. Any reordering here silently changes every mesa account
/// hash, so it is deliberately a mirror rather than a re-derivation.
impl<const N: usize> ToInputs for ZkAppAccountOf<N> {
    fn to_inputs(&self, inputs: &mut Inputs) {
        let Self {
            app_state,
            verification_key,
            zkapp_version,
            action_state,
            last_action_slot,
            proved_state,
            zkapp_uri,
        } = self;

        inputs.append(&Some(zkapp_uri));
        inputs.append_bool(*proved_state);
        inputs.append_u32(last_action_slot.as_u32());

        for fp in action_state {
            inputs.append_field(*fp);
        }

        inputs.append_u32(*zkapp_version);

        let vk_hash = verification_key
            .as_ref()
            .map(VerificationKeyWire::hash)
            .unwrap_or_else(VerificationKeyWire::dummy_hash);
        inputs.append_field(vk_hash);

        // the mesa delta: `N` field elements (32), where V2 has 8
        for fp in app_state {
            inputs.append_field(*fp);
        }
    }
}

impl<const N: usize> ZkAppAccountOf<N> {
    pub fn hash(&self) -> Fp {
        self.hash_with_param(&MINA_ZKAPP_ACCOUNT)
    }
}

/// The hash a non-zkApp account contributes in place of a zkApp -- the hash of the
/// *default* zkApp account. It depends on the state width, so mesa's differs from V2's.
///
/// Cached per width. A `static` inside a generic fn is *not* monomorphised per `N`, so the
/// widths are keyed explicitly rather than sharing one cell.
pub fn default_zkapp_hash_of<const N: usize>() -> Fp {
    static V2: Lazy<Fp> = Lazy::new(|| V2ZkAppAccount::default().hash());
    static MESA: Lazy<Fp> = Lazy::new(|| MesaZkAppAccount::default().hash());

    match N {
        V2_ZKAPP_STATE_SIZE => *V2,
        MESA_ZKAPP_STATE_SIZE => *MESA,
        _ => ZkAppAccountOf::<N>::default().hash(),
    }
}

/// The mesa default-zkApp hash.
pub fn default_mesa_zkapp_hash() -> Fp {
    default_zkapp_hash_of::<MESA_ZKAPP_STATE_SIZE>()
}

/// A ledger account whose zkApp state is `N` wide. Mirrors `mina-tree`'s `Account`; only
/// the zkApp differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountOf<const N: usize> {
    pub public_key: CompressedPubKey,
    pub token_id: TokenId,
    pub token_symbol: TokenSymbol,
    pub balance: Balance,
    pub nonce: Nonce,
    pub receipt_chain_hash: ReceiptChainHash,
    pub delegate: Option<CompressedPubKey>,
    pub voting_for: VotingFor,
    pub timing: Timing,
    pub permissions: Permissions<mina_tree::AuthRequired>,
    pub zkapp: Option<Box<ZkAppAccountOf<N>>>,
}

/// The mesa account.
pub type MesaAccount = AccountOf<MESA_ZKAPP_STATE_SIZE>;

/// The V2 account, for the control test.
pub type V2Account = AccountOf<V2_ZKAPP_STATE_SIZE>;

/// Field order is mina-tree's `impl ToInputs for Account`, verbatim. The zkApp is folded
/// to a single field, which is the only place mesa differs.
impl<const N: usize> ToInputs for AccountOf<N> {
    fn to_inputs(&self, inputs: &mut Inputs) {
        let Self {
            public_key,
            token_id,
            token_symbol,
            balance,
            nonce,
            receipt_chain_hash,
            delegate,
            voting_for,
            timing,
            permissions,
            zkapp,
        } = self;

        let field_zkapp = match zkapp.as_ref() {
            Some(zkapp) => zkapp.hash(),
            None => default_zkapp_hash_of::<N>(),
        };
        inputs.append(&field_zkapp);
        inputs.append(permissions);

        let TimingAsRecord {
            is_timed,
            initial_minimum_balance,
            cliff_time,
            cliff_amount,
            vesting_period,
            vesting_increment,
        } = timing.to_record();
        inputs.append_bool(is_timed);
        inputs.append_u64(initial_minimum_balance.as_u64());
        inputs.append_u32(cliff_time.as_u32());
        inputs.append_u64(cliff_amount.as_u64());
        inputs.append_u32(vesting_period.as_u32());
        inputs.append_u64(vesting_increment.as_u64());

        inputs.append_field(voting_for.0);

        let delegate = MyCow::borrow_or_else(delegate, CompressedPubKey::empty);
        inputs.append(delegate.as_ref());

        inputs.append_field(receipt_chain_hash.0);
        inputs.append_u32(nonce.as_u32());
        inputs.append_u64(balance.as_u64());

        // mina_base/account.ml caps the symbol at 6 bytes
        assert!(token_symbol.len() <= 6);
        inputs.append(token_symbol);

        inputs.append_field(token_id.0);
        inputs.append(public_key);
    }
}

impl<const N: usize> AccountOf<N> {
    pub fn hash(&self) -> Fp {
        self.hash_with_param(&MINA_ACCOUNT)
    }

    /// The empty account -- the leaf of every unoccupied slot in the ledger.
    ///
    /// The transaction version is picked from the state width, because the two travel
    /// together: width 8 is Berkeley (txn version 3), width 32 is mesa (txn version 4).
    /// It is hashed in via the default permissions, so getting it wrong corrupts every
    /// ledger root -- see [`MESA_TXN_VERSION`].
    pub fn empty() -> Self {
        let txn_version = match N {
            MESA_ZKAPP_STATE_SIZE => MESA_TXN_VERSION,
            _ => V2_TXN_VERSION,
        };

        Self::empty_with_txn_version(txn_version)
    }

    /// The empty account for an explicit transaction version.
    pub fn empty_with_txn_version(txn_version: u32) -> Self {
        Self {
            public_key: CompressedPubKey::empty(),
            token_id: TokenId::default(),
            token_symbol: TokenSymbol::default(),
            balance: Balance::zero(),
            nonce: Nonce::zero(),
            receipt_chain_hash: ReceiptChainHash::empty(),
            delegate: None,
            voting_for: VotingFor::dummy(),
            timing: Timing::Untimed,
            permissions: user_default_permissions(txn_version),
            zkapp: None,
        }
    }
}
