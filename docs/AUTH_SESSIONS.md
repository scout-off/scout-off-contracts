# Authentication sessions

The auth API associates each login with a session (`sid`). Access tokens carry
that session ID and expire after 15 minutes. A session owns its refresh-token
family; refresh token values are opaque, and only their SHA-256 hashes are
stored.

## Endpoints

- `POST /api/sep10/token` accepts `{ "sub": "...", "role": "..." }` and returns
  an access token (`token` and `accessToken`) plus a refresh token. This is the
  development token-issuance endpoint; production SEP-10 challenge validation
  must happen before invoking session creation.
- `POST /auth/refresh` accepts `{ "refreshToken": "..." }`. Successful rotation
  returns a new access and refresh token. Reusing an already-rotated token
  revokes the whole family and returns `401 { "error": "refresh_token_reused" }`.
  The conditional refresh-token update is atomic, so only one concurrent
  rotation can succeed.
- `POST /auth/logout` requires a bearer access token and revokes its session
  and current access-token `jti`.
- `POST /auth/logout-all` requires a bearer access token and revokes every
  session for its `sub`, including each family's refresh tokens.
- `POST /api/admin/tokens/revoke` requires an admin bearer token and accepts
  exactly one of `{ "sid": "..." }` or `{ "account": "..." }` to revoke
  sessions. The legacy `{ "token": "..." }` form remains available for
  revoking a single token issued before sessions existed.

Protected requests check both the `jti` denylist and session revocation state.
Session state is cached for up to two seconds to limit database reads; local
logout/revocation clears the relevant cache entries immediately. Tokens issued
before session support have no `sid` and remain governed by their `jti` until
their existing expiry (new access tokens expire within 15 minutes).

Apply `migrations/008_session_management.sql` before running the API and keep
the Prisma schema synchronized with the database.
Run `npm run check:auth-schema` with `DATABASE_URL` configured to check for
missing session tables, columns, or the refresh-token foreign key.
