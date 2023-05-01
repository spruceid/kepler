pub trait Hasher {
    const OUTPUT_SIZE: usize;
    fn update(&mut self, input: &[u8]);
    fn finalize(&mut self) -> [u8; Self::OUTPUT_SIZE];
}
