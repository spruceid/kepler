pub fn make_orbit_id_pkh_eip155(address: String, chain_id: u32, name: Option<String>) -> String {
    make_orbit_id(format!("pkh:eip155:{chain_id}:{address}"), name)
}

fn make_orbit_id(did_suffix: String, name: Option<String>) -> String {
    format!(
        "kepler:{did_suffix}://{}",
        name.unwrap_or_else(|| String::from("default"))
    )
}
