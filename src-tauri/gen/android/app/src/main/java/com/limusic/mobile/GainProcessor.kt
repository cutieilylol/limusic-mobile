package com.limusic.mobile

import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor.AudioFormat
import androidx.media3.common.audio.BaseAudioProcessor
import androidx.media3.common.util.UnstableApi
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.abs
import kotlin.math.exp
import kotlin.math.max
import kotlin.math.pow

/**
 * Loudness normalization: the per-track gain the Rust side computes from YouTube's loudness data,
 * applied ahead of ExoPlayer's own speed/pitch stage. The desktop build does this with mpv's
 * `volume=<g>dB` filter.
 *
 * A positive gain gets a peak limiter whose ceiling is full scale, so it only touches samples that
 * would otherwise clip (the desktop `alimiter=limit=1`): instant attack, so nothing gets past it,
 * and a 150 ms release so the gain comes back smoothly after a peak.
 *
 * The gain can change while audio is flowing (a gapless advance sets the next track's gain), so
 * it is read per buffer and never needs the sink to reconfigure.
 */
@OptIn(UnstableApi::class)
class GainProcessor : BaseAudioProcessor() {
    @Volatile
    private var linear = 1f
    private var peak = 0f
    private var release = 0.9999f

    fun setDb(db: Double) {
        linear = if (db.isFinite()) 10.0.pow(db / 20.0).toFloat() else 1f
        android.util.Log.i("GainProcessor", "gain ${"%.2f".format(db)} dB")
    }

    override fun onConfigure(inputAudioFormat: AudioFormat): AudioFormat {
        if (inputAudioFormat.encoding != C.ENCODING_PCM_16BIT &&
            inputAudioFormat.encoding != C.ENCODING_PCM_FLOAT
        ) {
            android.util.Log.w("GainProcessor", "inactive for encoding ${inputAudioFormat.encoding}")
            return AudioFormat.NOT_SET
        }
        android.util.Log.i("GainProcessor", "active: $inputAudioFormat")
        val samplesPerSec = max(1, inputAudioFormat.sampleRate * inputAudioFormat.channelCount)
        release = exp(-1.0 / (0.150 * samplesPerSec)).toFloat()
        return inputAudioFormat
    }

    override fun queueInput(inputBuffer: ByteBuffer) {
        val remaining = inputBuffer.remaining()
        if (remaining == 0) return
        val out = replaceOutputBuffer(remaining).order(ByteOrder.nativeOrder())
        val g = linear
        val limit = g > 1f
        when (inputAudioFormat.encoding) {
            C.ENCODING_PCM_16BIT -> {
                while (inputBuffer.remaining() >= 2) {
                    var x = inputBuffer.short / 32768f * g
                    if (limit) x = limited(x)
                    out.putShort((x * 32767f).coerceIn(-32768f, 32767f).toInt().toShort())
                }
            }
            else -> {
                while (inputBuffer.remaining() >= 4) {
                    var x = inputBuffer.float * g
                    if (limit) x = limited(x)
                    out.putFloat(x)
                }
            }
        }
        inputBuffer.position(inputBuffer.limit())
        out.flip()
    }

    private fun limited(x: Float): Float {
        val a = abs(x)
        peak = if (a > peak) a else peak * release + a * (1 - release)
        return if (peak > 1f) x / peak else x
    }

    override fun onReset() {
        peak = 0f
    }
}
