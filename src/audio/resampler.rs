use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

pub struct AudioResampler {
    resampler: SincFixedIn<f64>,
    source_rate: u32,
    target_rate: u32,
    channels: usize,
    chunk_size: usize,
    input_buffer: Vec<Vec<f64>>,
    temp_channels: Vec<Vec<f64>>,
}

impl AudioResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Result<Self, String> {
        if source_rate == target_rate {
            return Err("source and target rate are the same".into());
        }

        let chunk_size = 1024;

        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            oversampling_factor: 16,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::Hann,
        };
        let ratio = target_rate as f64 / source_rate as f64;
        let max_ratio_factor = ratio.max(1.0) * 2.0;
        let resampler =
            SincFixedIn::<f64>::new(ratio, max_ratio_factor, params, chunk_size, channels)
                .map_err(|e| format!("Failed to create resampler: {:?}", e))?;

        log::info!(
            "Resampler created: {}Hz -> {}Hz, {}ch, chunk={}",
            source_rate,
            target_rate,
            channels,
            chunk_size
        );

        Ok(Self {
            resampler,
            source_rate,
            target_rate,
            channels,
            chunk_size,
            input_buffer: vec![Vec::with_capacity(chunk_size * 2); channels],
            temp_channels: vec![Vec::with_capacity(chunk_size); channels],
        })
    }

    pub fn resample(&mut self, input: &[i16]) -> Result<Vec<i16>, String> {
        if self.source_rate == self.target_rate {
            return Ok(input.to_vec());
        }

        for (i, &sample) in input.iter().enumerate() {
            let ch = i % self.channels;
            self.input_buffer[ch].push(sample as f64 / i16::MAX as f64);
        }

        let mut result = Vec::new();

        while self.input_buffer[0].len() >= self.chunk_size {
            for (i, buf) in self.input_buffer.iter_mut().enumerate() {
                self.temp_channels[i].clear();
                self.temp_channels[i].extend_from_slice(&buf[..self.chunk_size]);
                buf.drain(..self.chunk_size);
            }

            let channels_data: Vec<&[f64]> = self
                .temp_channels
                .iter()
                .map(|buf| buf.as_slice())
                .collect();

            let output = self
                .resampler
                .process(&channels_data, None)
                .map_err(|e| format!("Resample error: {:?}", e))?;

            let output_frames = output[0].len();
            result.reserve(output_frames * self.channels);

            for frame in 0..output_frames {
                for ch in 0..self.channels {
                    let sample = output[ch][frame].clamp(-1.0, 1.0) * i16::MAX as f64;
                    result.push(sample as i16);
                }
            }
        }

        Ok(result)
    }
}
