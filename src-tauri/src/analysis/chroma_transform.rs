//! 72-band Direct Spectral Kernel chroma transform.
//!
//! Ported from libKeyFinder's `Chromatransform` (Ibrahim Sha'ath, 2011-2015).
//! This is a constant-Q transform approximation that maps an FFT magnitude
//! spectrum to 72 chroma bands (6 octaves × 12 semitones, C1–B6).
//!
//! Unlike the simple 12-bin MIDI mapper, this preserves octave information
//! and uses cosine windowing for robustness to tuning variations.

use rustfft::num_complex::Complex;

use super::FFT_SIZE;

/// Frequencies for the 72 chroma bands, C1 through B6.
/// From libKeyFinder `constants.cpp`.
const FREQUENCIES: [f64; 72] = [
    32.7031956625748,  34.647828872109,   36.708095989676,   38.8908729652601,
    41.2034446141088,  43.6535289291255,  46.2493028389543,  48.9994294977187,
    51.9130871974932,  55.0,              58.2704701897613,  61.7354126570155,
    65.4063913251497,  69.2956577442181,  73.4161919793519,  77.7817459305203,
    82.4068892282175,  87.307057858251,   92.4986056779087,  97.9988589954374,
    103.826174394986,  110.0,             116.540940379523,  123.470825314031,
    130.812782650299,  138.591315488436,  146.832383958704,  155.563491861041,
    164.813778456435,  174.614115716502,  184.997211355817,  195.997717990875,
    207.652348789973,  220.0,             233.081880759045,  246.941650628062,
    261.625565300599,  277.182630976872,  293.664767917408,  311.126983722081,
    329.62755691287,   349.228231433004,  369.994422711635,  391.99543598175,
    415.304697579946,  440.000000000001,  466.163761518091,  493.883301256125,
    523.251130601198,  554.365261953745,  587.329535834816,  622.253967444163,
    659.255113825741,  698.456462866009,  739.98884542327,   783.9908719635,
    830.609395159892,  880.000000000002,  932.327523036182,  987.76660251225,
    1046.5022612024,   1108.73052390749,  1174.65907166963,  1244.50793488833,
    1318.51022765148,  1396.91292573202,  1479.97769084654,  1567.981743927,
    1661.21879031978,  1760.0,            1864.65504607236,  1975.5332050245,
];

/// Q-factor stretch parameter. From libKeyFinder `constants.h`.
const DIRECT_SK_STRETCH: f64 = 0.8;

/// Precomputed Direct Spectral Kernel for mapping FFT bins to 72 chroma bands.
///
/// For each of the 72 bands, stores:
/// - `fft_bin_offset`: first FFT bin that contributes to this band
/// - `kernel`: weights for each contributing FFT bin
pub struct ChromaTransform {
    /// First contributing FFT bin index for each band.
    fft_bin_offsets: [usize; 72],
    /// Kernel weights for each band. `kernels[band][k]` is the weight for
    /// FFT bin `fft_bin_offsets[band] + k`.
    kernels: [Vec<f64>; 72],
}

impl ChromaTransform {
    /// Build the Direct Spectral Kernel for a given sample rate.
    ///
    /// Panics if the sample rate would cause analysis frequencies to exceed
    /// Nyquist, or if low-end resolution is insufficient.
    pub fn new(frame_rate: usize) -> Self {
        assert!(frame_rate > 0, "Frame rate must be > 0");

        let last_freq = FREQUENCIES[71];
        assert!(
            last_freq <= frame_rate as f64 / 2.0,
            "Analysis frequencies over Nyquist"
        );

        let bin_width = frame_rate as f64 / FFT_SIZE as f64;
        assert!(
            bin_width <= FREQUENCIES[1] - FREQUENCIES[0],
            "Insufficient low-end resolution"
        );

        let my_q_factor = DIRECT_SK_STRETCH * (2.0f64.powf(1.0 / 12.0) - 1.0);

        let mut fft_bin_offsets = [0usize; 72];
        let mut kernels: [Vec<f64>; 72] = [
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
            Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
        ];

        for i in 0..72 {
            let centre_of_window = FREQUENCIES[i] * FFT_SIZE as f64 / frame_rate as f64;
            let width_of_window = centre_of_window * my_q_factor;
            let beginning_of_window = centre_of_window - (width_of_window / 2.0);
            let end_of_window = beginning_of_window + width_of_window;

            let mut sum_of_coefficients = 0.0;

            fft_bin_offsets[i] = beginning_of_window.ceil() as usize;
            let end_bin = end_of_window.floor() as usize;

            for fft_bin in fft_bin_offsets[i]..=end_bin {
                let coefficient = kernel_window(fft_bin as f64 - beginning_of_window, width_of_window);
                sum_of_coefficients += coefficient;
                kernels[i].push(coefficient);
            }

            // Normalise by sum of coefficients and frequency; models CQT closely.
            let freq = FREQUENCIES[i];
            for j in 0..kernels[i].len() {
                kernels[i][j] = kernels[i][j] / sum_of_coefficients * freq;
            }
        }

        Self { fft_bin_offsets, kernels }
    }

    /// Extract a 72-element chroma vector from FFT output (complex).
    ///
    /// `fft_output` should be the complex FFT result for the first `FFT_SIZE/2`
    /// bins (positive frequencies only).
    pub fn chroma_vector(&self, fft_output: &[Complex<f32>]) -> [f64; 72] {
        let mut chroma = [0.0f64; 72];
        for i in 0..72 {
            let mut sum = 0.0;
            for j in 0..self.kernels[i].len() {
                let bin = self.fft_bin_offsets[i] + j;
                if bin < fft_output.len() {
                    let magnitude = {
                        let c = fft_output[bin];
                        ((c.re * c.re + c.im * c.im) as f64).sqrt()
                    };
                    sum += magnitude * self.kernels[i][j];
                }
            }
            chroma[i] = sum;
        }
        chroma
    }

    /// Extract a 72-element chroma vector from pre-computed magnitude spectrum.
    ///
    /// Use this when the magnitude spectrogram has already been modified
    /// (e.g. after HPSS). `magnitudes` should have at least `FFT_SIZE/2` elements.
    pub fn chroma_vector_from_magnitudes(&self, magnitudes: &[f64]) -> [f64; 72] {
        let mut chroma = [0.0f64; 72];
        for i in 0..72 {
            let mut sum = 0.0;
            for j in 0..self.kernels[i].len() {
                let bin = self.fft_bin_offsets[i] + j;
                if bin < magnitudes.len() {
                    sum += magnitudes[bin] * self.kernels[i][j];
                }
            }
            chroma[i] = sum;
        }
        chroma
    }
}

/// Cosine window function: `1 - cos(2π·n/N)`.
/// This is the kernel window from libKeyFinder, distinct from standard
/// Hann/Hamming/Blackman windows.
fn kernel_window(n: f64, n_total: f64) -> f64 {
    1.0 - (2.0 * std::f64::consts::PI * n / n_total).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_without_panic_at_22050hz() {
        let ct = ChromaTransform::new(22050);
        // All 72 bands should have at least one kernel weight.
        for i in 0..72 {
            assert!(!ct.kernels[i].is_empty(), "Band {} has empty kernel", i);
        }
    }

    #[test]
    fn chroma_vector_produces_72_elements() {
        let ct = ChromaTransform::new(22050);
        let fft_out = vec![Complex::new(0.0f32, 0.0); FFT_SIZE / 2];
        let cv = ct.chroma_vector(&fft_out);
        assert_eq!(cv.len(), 72);
    }

    #[test]
    #[should_panic(expected = "Analysis frequencies over Nyquist")]
    fn panics_if_nyquist_violated() {
        // With sample rate 1000 Hz, Nyquist = 500 Hz, but last freq = 1975 Hz.
        ChromaTransform::new(1000);
    }
}
