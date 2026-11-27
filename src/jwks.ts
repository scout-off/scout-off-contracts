import { KeyObject } from "crypto";
import { SigningKeys } from "./keys";

interface JwkKey {
  kty: string;
  crv?: string;
  x?: string;
  use: string;
  kid: string;
  alg: string;
}

/** Convert a Node.js Ed25519 public KeyObject to JWK format. */
function ed25519PublicKeyToJwk(kid: string, key: KeyObject): JwkKey {
  const jwk = key.export({ format: "jwk" }) as {
    kty: string;
    crv: string;
    x: string;
  };
  return {
    kty: jwk.kty,
    crv: jwk.crv,
    x: jwk.x,
    use: "sig",
    kid,
    alg: "EdDSA",
  };
}

/** Build the JWKS payload from the loaded signing keys. */
export function buildJwks(signingKeys: SigningKeys): { keys: JwkKey[] } {
  const keys: JwkKey[] = [];
  for (const [kid, pub] of signingKeys.publicKeys) {
    keys.push(ed25519PublicKeyToJwk(kid, pub));
  }
  return { keys };
}
