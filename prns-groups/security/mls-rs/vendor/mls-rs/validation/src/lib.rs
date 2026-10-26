#![no_std]
#[cfg(target_os = "none")]
fn missing_entropy(_: &mut [u8]) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}
#[cfg(target_os = "none")]
getrandom::register_custom_getrandom!(missing_entropy);
