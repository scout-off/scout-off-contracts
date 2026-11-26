# Request validation

Backend request bodies and query strings are validated with `zod` schemas in
`src/requestValidation.ts` through the `validate({ query?, body? })` middleware.

- Schemas: `AccountQuery`, `VerifyBody` (`transaction` base64, max 10 KB, checked before XDR parsing), `RefreshBody`, `RevokeBody` (JWT-shaped `token`).
- Unknown keys are **stripped**; handlers only see declared fields via the parsed `req.query` / `req.body`.
- Failures return `400`:

```json
{ "error": "invalid_request", "details": [{ "path": "account", "message": "invalid Stellar account" }] }
```

Authentication failures (`401`) keep their existing stable error codes.
