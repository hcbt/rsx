//! Host DAC for Machine PCM. Not the guest clock — that is CPU_HZ vs wall time.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample};

/// Emergency cap (1 s). The Debugger must not use this as a speed control.
const MAX_FRAMES: usize = 44_100;

struct PcmBuf {
    q: VecDeque<(i16, i16)>,
}

impl PcmBuf {
    fn new() -> Self {
        Self {
            q: VecDeque::with_capacity(2048),
        }
    }

    fn push_interleaved(&mut self, pcm: &[i16]) {
        for c in pcm.chunks_exact(2) {
            if self.q.len() >= MAX_FRAMES {
                break;
            }
            self.q.push_back((c[0], c[1]));
        }
    }

    fn pop(&mut self) -> (i16, i16) {
        self.q.pop_front().unwrap_or((0, 0))
    }

    fn len(&self) -> usize {
        self.q.len()
    }
}

pub struct Output {
    buf: Arc<Mutex<PcmBuf>>,
    _stream: cpal::Stream,
}

impl Output {
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no host audio output device".to_string())?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("audio config: {e}"))?;
        let mut config = supported.config();
        config.sample_rate = cpal::SampleRate(44100);
        config.channels = 2;
        let buf = Arc::new(Mutex::new(PcmBuf::new()));
        let err_fn = |e| eprintln!("audio stream: {e}");
        let stream = match open(&device, &config, supported.sample_format(), buf.clone(), err_fn) {
            Ok(s) => s,
            Err(_) => {
                let fallback = supported.config();
                open(
                    &device,
                    &fallback,
                    supported.sample_format(),
                    buf.clone(),
                    err_fn,
                )?
            }
        };
        stream.play().map_err(|e| format!("audio play: {e}"))?;
        Ok(Self {
            buf,
            _stream: stream,
        })
    }

    pub fn push(&self, pcm: &[i16]) {
        self.buf.lock().expect("audio mutex").push_interleaved(pcm);
    }
}

fn open(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    buf: Arc<Mutex<PcmBuf>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String> {
    let host_rate = config.sample_rate.0;
    let channels = config.channels as usize;
    match format {
        cpal::SampleFormat::F32 => build::<f32>(device, config, buf, host_rate, channels, err_fn),
        cpal::SampleFormat::I16 => build::<i16>(device, config, buf, host_rate, channels, err_fn),
        cpal::SampleFormat::U16 => build::<u16>(device, config, buf, host_rate, channels, err_fn),
        other => Err(format!("unsupported sample format {other}")),
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buf: Arc<Mutex<PcmBuf>>,
    host_rate: u32,
    channels: usize,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: Sample + FromSample<f32> + cpal::SizedSample,
{
    let mut acc = 0u32;
    let mut cur = (0i16, 0i16);
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut b = buf.lock().expect("audio mutex");
                for frame in out.chunks_mut(channels) {
                    acc += 44100;
                    while acc >= host_rate {
                        acc -= host_rate;
                        cur = b.pop();
                    }
                    let l = f32::from(cur.0) / 32768.0;
                    let r = f32::from(cur.1) / 32768.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_keeps_stereo_pairs() {
        let mut b = PcmBuf::new();
        b.push_interleaved(&[1, 2, 3, 4, 5]);
        assert_eq!(b.len(), 2);
        assert_eq!(b.pop(), (1, 2));
        assert_eq!(b.pop(), (3, 4));
        assert_eq!(b.pop(), (0, 0));
    }

    #[test]
    fn full_buffer_keeps_the_oldest_queued_frame() {
        let mut b = PcmBuf::new();
        let many = vec![7, 8].repeat(MAX_FRAMES + 10);
        b.push_interleaved(&many);
        assert_eq!(b.len(), MAX_FRAMES);
        assert_eq!(b.pop(), (7, 8));
    }
}
