declare module "libsodium-wrappers" {
  const sodium: {
    ready: Promise<void>;
    crypto_sign_ed25519_pk_to_curve25519(publicKey: Uint8Array): Uint8Array;
    crypto_sign_seed_keypair(seed: Uint8Array): {
      publicKey: Uint8Array;
      privateKey: Uint8Array;
    };
    crypto_sign_ed25519_sk_to_curve25519(secretKey: Uint8Array): Uint8Array;
    crypto_box_seal(message: Uint8Array, publicKey: Uint8Array): Uint8Array;
    crypto_box_seal_open(
      ciphertext: Uint8Array,
      publicKey: Uint8Array,
      secretKey: Uint8Array
    ): Uint8Array;
  };

  export default sodium;
}
