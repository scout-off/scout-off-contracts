use scoutchain_shared_types::AdminError;
use soroban_sdk::contracterror;

/// Append-only: do not renumber existing variants. See docs/CONTRIBUTING.md.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ScoutChainError {
    // ── Initialization & lifecycle ──
    /// `initialize` called more than once.
    AlreadyInitialized = 1,
    /// Operation before `initialize`.
    NotInitialized = 2,
    /// Circuit breaker is active.
    ContractPaused = 9,

    // ── Authorization ──
    /// Unregistered account approving milestone.
    ValidatorNotAuthorized = 4,
    /// Wrong account for a privileged operation.
    Unauthorized = 10,

    // ── Registration & lookup ──
    /// Wallet already has a profile for this role.
    AlreadyRegistered = 8,
    /// Invalid `player_id`.
    PlayerNotFound = 3,
    /// Invalid `scout_id`.
    ScoutNotFound = 12,
    /// Player registration cap reached.
    PlayerCapReached = 15,

    // ── Business logic ──
    /// Skipping or reversing a level.
    InvalidProgressTransition = 5,
    /// Scout has no subscription.
    ScoutNotSubscribed = 6,
    /// Underpaying contact fee.
    InsufficientFee = 7,

    // ── Validation & arithmetic ──
    /// Field too long, bad hash count, or empty value.
    InvalidInput = 13,
    /// Counter or fee arithmetic overflowed.
    Overflow = 11,

    // ── Admin transfer ──
    /// `accept_admin` called before an admin transfer was proposed.
    PendingAdminNotSet = 14,

    // ── Rate limiting ──
    /// Caller attempted to register again before the cooldown period elapsed.
    RegistrationCooldown = 16,

    // ── Archival recovery ──
    /// `restore_player_record` targeted a player entry whose archival grace
    /// period has fully elapsed (evicted, not merely archived) and is
    /// unrecoverable.
    PlayerRecordEvicted = 17,
    /// `restore_scout_record` targeted a scout entry that has been fully
    /// evicted and is unrecoverable.
    ScoutRecordEvicted = 18,
}

impl AdminError for ScoutChainError {
    fn not_initialized() -> Self {
        ScoutChainError::NotInitialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_ERROR_CODES: &[(&str, u32)] = &[
        ("AlreadyInitialized", 1),
        ("NotInitialized", 2),
        ("ContractPaused", 9),
        ("ValidatorNotAuthorized", 4),
        ("Unauthorized", 10),
        ("AlreadyRegistered", 8),
        ("PlayerNotFound", 3),
        ("ScoutNotFound", 12),
        ("PlayerCapReached", 15),
        ("InvalidProgressTransition", 5),
        ("ScoutNotSubscribed", 6),
        ("InsufficientFee", 7),
        ("InvalidInput", 13),
        ("Overflow", 11),
        ("PendingAdminNotSet", 14),
        ("RegistrationCooldown", 16),
        ("PlayerRecordEvicted", 17),
        ("ScoutRecordEvicted", 18),
    ];

    #[test]
    fn scout_chain_error_discriminants_are_stable() {
        assert_eq!(ScoutChainError::AlreadyInitialized as u32, 1);
        assert_eq!(ScoutChainError::NotInitialized as u32, 2);
        assert_eq!(ScoutChainError::PlayerNotFound as u32, 3);
        assert_eq!(ScoutChainError::ValidatorNotAuthorized as u32, 4);
        assert_eq!(ScoutChainError::InvalidProgressTransition as u32, 5);
        assert_eq!(ScoutChainError::ScoutNotSubscribed as u32, 6);
        assert_eq!(ScoutChainError::InsufficientFee as u32, 7);
        assert_eq!(ScoutChainError::AlreadyRegistered as u32, 8);
        assert_eq!(ScoutChainError::ContractPaused as u32, 9);
        assert_eq!(ScoutChainError::Unauthorized as u32, 10);
        assert_eq!(ScoutChainError::Overflow as u32, 11);
        assert_eq!(ScoutChainError::ScoutNotFound as u32, 12);
        assert_eq!(ScoutChainError::InvalidInput as u32, 13);
        assert_eq!(ScoutChainError::PendingAdminNotSet as u32, 14);
        assert_eq!(ScoutChainError::PlayerCapReached as u32, 15);
        assert_eq!(ScoutChainError::RegistrationCooldown as u32, 16);
        assert_eq!(ScoutChainError::PlayerRecordEvicted as u32, 17);
        assert_eq!(ScoutChainError::ScoutRecordEvicted as u32, 18);
    }

    /// The two variants this file previously conflated.
    ///
    /// `PlayerCapReached` is a hard stop — the platform is full.
    /// `RegistrationCooldown` means "try again later". A client switching on
    /// the numeric code shows the wrong thing to the user if these two ever
    /// swap or merge, so both are pinned here as well as in the table above,
    /// with the reasoning attached.
    ///
    /// Note on what this does *not* need to guard: two variants sharing one
    /// discriminant is rejected by rustc itself (E0081, "discriminant value
    /// assigned more than once") even for a fieldless `#[repr(u32)]` enum, so
    /// a collision cannot reach a green build. The failure mode that *can*
    /// slip through is renumbering — moving an already-shipped code to a new
    /// variant, which compiles cleanly and silently changes what every
    /// existing client sees. That is what these assertions exist for.
    #[test]
    fn player_cap_and_cooldown_are_distinguishable() {
        assert_eq!(ScoutChainError::PlayerCapReached as u32, 15);
        assert_eq!(ScoutChainError::RegistrationCooldown as u32, 16);
        assert_ne!(
            ScoutChainError::PlayerCapReached as u32,
            ScoutChainError::RegistrationCooldown as u32
        );
    }

    #[test]
    fn scout_chain_error_source_matches_expected_unique_codes() {
        let mut in_error_enum = false;
        let mut expected_index = 0usize;
        let mut seen_codes = [false; 256];

        for raw_line in include_str!("errors.rs").lines() {
            let line = raw_line.trim();

            if line.starts_with("pub enum ScoutChainError") {
                in_error_enum = true;
                continue;
            }

            if !in_error_enum {
                continue;
            }

            if line.starts_with('}') {
                break;
            }

            if line.is_empty()
                || line.starts_with("//")
                || line.starts_with("///")
                || !line.ends_with(',')
            {
                continue;
            }

            let (variant, discriminant) = line
                .trim_end_matches(',')
                .split_once('=')
                .expect("ScoutChainError variants must use explicit discriminants");
            let assigned_code = discriminant
                .trim()
                .parse::<u32>()
                .expect("ScoutChainError discriminants must be u32 literals");
            let (expected_variant, expected_code) = EXPECTED_ERROR_CODES
                .get(expected_index)
                .copied()
                .expect("ScoutChainError has an unpinned variant");

            assert_eq!(variant.trim(), expected_variant);
            assert_eq!(assigned_code, expected_code);

            let code_index = assigned_code as usize;
            assert!(
                code_index < seen_codes.len(),
                "ScoutChainError code {assigned_code} exceeds the test range"
            );
            assert!(
                !seen_codes[code_index],
                "ScoutChainError code {assigned_code} is assigned more than once"
            );
            seen_codes[code_index] = true;
            expected_index += 1;
        }

        assert_eq!(
            expected_index,
            EXPECTED_ERROR_CODES.len(),
            "ScoutChainError is missing a pinned variant"
        );
    }
}
