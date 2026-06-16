#[allow(dead_code)]
pub struct Resampler {
    source_rate: u32,
    target_rate: u32,
    accumulator: f64,
}

#[allow(dead_code)]
impl Resampler {
    #[allow(dead_code)]
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            source_rate,
            target_rate,
            accumulator: 0.0,
        }
    }

    #[allow(dead_code)]
    pub fn resample(&mut self, input: &[i16]) -> Vec<i16> {
        if self.source_rate == self.target_rate {
            return input.to_vec();
        }

        let ratio = self.source_rate as f64 / self.target_rate as f64;
        let output_len = (input.len() as f64 / ratio) as usize;
        let mut output = Vec::with_capacity(output_len);

        while (output.len() as f64) * ratio < input.len() as f64 {
            let src_idx = self.accumulator;
            let idx = src_idx as usize;

            if idx + 1 < input.len() {
                let frac = src_idx - idx as f64;
                let sample =
                    input[idx] as f64 * (1.0 - frac) + input[idx + 1] as f64 * frac;
                output.push(sample as i16);
            } else if idx < input.len() {
                output.push(input[idx]);
            }

            self.accumulator += ratio;
        }

        self.accumulator -= input.len() as f64;
        if self.accumulator < 0.0 {
            self.accumulator = 0.0;
        }

        output
    }
}
