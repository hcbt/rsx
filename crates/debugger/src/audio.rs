//! Host playback of Machine PCM. The Machine does not know about this.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample};

pub struct Output {
    buf: Arc<Mutex<VecDeque<i16>>>,
    _stream: cpal::Stream,
}

impl Output {
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no host audio output device".to_string())?;
        let cfg = device
            .default_output_config()
            .map_err(|e| format!("audio config: {e}"))?;
        let sample_rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let buf = Arc::new(Mutex::new(VecDeque::<i16>::new()));
        let shared = buf.clone();
        let err_fn = |e| eprintln!("audio stream: {e}");
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &cfg.config(), shared, sample_rate, channels, err_fn)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &cfg.config(), shared, sample_rate, channels, err_fn)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &cfg.config(), shared, sample_rate, channels, err_fn)?,
            other => return Err(format!("unsupported sample format {other}")),
        };
        stream.play().map_err(|e| format!("audio play: {e}"))?;
        Ok(Self {
            buf,
            _stream: stream,
        })
    }

    pub fn push(&self, pcm: &[i16]) {
        let mut b = self.buf.lock().expect("audio mutex");
        b.extend(pcm.iter().copied());
        const CAP: usize = 44_100;
        while b.len() > CAP {
            b.pop_front();
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buf: Arc<Mutex<VecDeque<i16>>>,
    host_rate: u32,
    channels: usize,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: Sample + FromSample<f32> + cpal::SizedSample,
{
    let mut acc = 0u32;
    let mut cur_l = 0i16;
    let mut cur_r = 0i16;
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut b = buf.lock().expect("audio mutex");
                for frame in out.chunks_mut(channels) {
                    acc += 44100;
                    while acc >= host_rate {
                        acc -= host_rate;
                        cur_l = b.pop_front().unwrap_or(cur_l);
                        cur_r = b.pop_front().unwrap_or(cur_r);
                    }
                    let l = f32::from(cur_l) / 32768.0;
                    let r = f32::from(cur_r) / 32768.0;
                    if !frame.is_empty() {
                        frame[0] = T::from_sample(l);
                    }
                    if frame.len() > 1 {
                        frame[1] = T::from_sample(r);
                    }
                    for s in frame.iter_mut().skip(2) {
                        *s = T::from_sample(0.0);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("audio stream: {e}"))
}
