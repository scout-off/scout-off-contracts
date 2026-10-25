-- Migration 013: Scout deregistration / GDPR erasure (issue #1641)
--
-- registration.deregister_scout(scout_id) erases a scout's profile on-chain
-- and emits `scout_deregistered` (topics: admin; data: (scout_id, wallet)).
-- On that event the indexer must DELETE the matching `scouts` row
-- (WHERE scout_id = $1) so no personal data outlives the on-chain record.
--
-- scout_access keeps wallet-keyed Subscription / ContactRecord / grant entries
-- (append-only audit data, not profile data); those rows are retained.

COMMENT ON TABLE scouts IS
    'Mirrors registration scout profiles. Rows are deleted on scout_deregistered (GDPR erasure, issue #1641).';
