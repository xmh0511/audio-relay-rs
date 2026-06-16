use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

pub struct AudioResampler {
    resampler: SincFixedIn<f64>,
    source_rate: u32,
    target_rate: u32,
    channels: usize,
}

impl AudioResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Result<Self, String> {
        if source_rate == target_rate {
            return Err("source and target rate are the same".into());
        }

        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            oversampling_factor: 16,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::Hann,
        };

        let resampler = SincFixedIn::<f64>::new(
            source_rate as f64 / target_rate as f64,
            2.0,
            params,
            1024,
            channels,
        )
        .map_err(|e| format!("Failed to create resampler: {:?}", e))?;

        log::info!(
            "Resampler created: {}Hz -> {}Hz, {}ch",
            source_rate,
            target_rate,
            channels
        );

        Ok(Self {
            resampler,
            source_rate,
            target_rate,
            channels,
        })
    }

    pub fn resample(&mut self, input: &[i16]) -> Result<Vec<i16>, String> {
        if self.source_rate == self.target_rate {
            return Ok(input.to_vec());
        }

        let frames_per_channel = input.len() / self.channels;

        let mut channels_data: Vec<Vec<f64>> =
            vec![Vec::with_capacity(frames_per_channel); self.channels];

        for (i, &sample) in input.iter().enumerate() {
            let ch = i % self.channels;
            channels_data[ch].push(sample as f64 / i16::MAX as f64);
        }

        let output = self
            .resampler
            .process(&channels_data, None)
            .map_err(|e| format!("Resample error: {:?}", e))?;

        let output_frames = output[0].len();
        let mut result = Vec::with_capacity(output_frames * self.channels);

        for frame in 0..output_frames {
            for ch in 0..self.channels {
                let sample = output[ch][frame].clamp(-1.0, 1.0) * i16::MAX as f64;
                result.push(sample as i16);
            }
        }

        Ok(result)
    }

    #[allow(dead_code)]
    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    #[allow(dead_code)]
    pub fn target_rate(&self) -> u32 {
        self.target_rate
    }
}
