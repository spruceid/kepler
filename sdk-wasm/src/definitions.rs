use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TS_DEF: &'static str = r#"
/**
 * Configuration object for starting a Kepler session.
 */
export type SessionConfig = {
  /** Actions that the session key will be permitted to perform, organized by service and path */
  actions: { [service: string]: { [key: string]: string[] }},
  /** Ethereum address. */
  address: string,
  /** Chain ID. */
  chainId: number,
  /** Domain of the webpage. */
  domain: string,
  /** Current time for SIWE message. */
  issuedAt: string,
  /** The orbit that is the target resource of the delegation. */
  orbitId: string,
  /** The earliest time that the session will be valid from. */
  notBefore?: string,
  /** The latest time that the session will be valid until. */
  expirationTime: string,
  /** Optional parent delegations to inherit and attenuate */
  parents?: string[]
  /** Optional jwk to delegate to */
  jwk?: object
}
"#;

#[wasm_bindgen(typescript_custom_section)]
const TS_DEF: &'static str = r#"
/**
 * A Kepler session.
 */
export type Session = {
  /** The delegation from the user to the session key. */
  delegationHeader: { Authorization: string },
  /** The delegation reference from the user to the session key. */
  delegationCid: string,
  /** The session key. */
  jwk: object,
  /** The orbit that the session key is permitted to perform actions against. */
  orbitId: string,
  /** The verification method of the session key. */
  verificationMethod: string,
}
"#;

#[wasm_bindgen(typescript_custom_section)]
const TS_DEF: &'static str = r#"
/**
 * Configuration object for generating a Orbit Host Delegation SIWE message.
 */
export type HostConfig = {
  /** Ethereum address. */
  address: string,
  /** Chain ID. */
  chainId: number,
  /** Domain of the webpage. */
  domain: string,
  /** Current time for SIWE message. */
  issuedAt: string,
  /** The orbit that is the target resource of the delegation. */
  orbitId: string,
  /** The peer that is the target/invoker in the delegation. */
  peerId: string,
}
"#;
