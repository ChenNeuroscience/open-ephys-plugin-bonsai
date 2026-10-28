#include "SampleWriter.h"

namespace CameraTracker {

    // Gaps longer than this many samples are treated as a lost source rather than filled.
    constexpr double maxGapSamples = 1000;

    SampleWriter::SampleWriter(DataBuffer* dataBuffer_, int numValues, double sampleRate_, QualityInfo& qualityInfo_)
        : dataBuffer(dataBuffer_), numChannels(numValues + 1), sampleRate(sampleRate_), qualityInfo(qualityInfo_)
    {
    }

    bool SampleWriter::write(double timestamp, const float* values)
    {
        if (nSamples == 0) {
            firstTimestamp = timestamp;
        } else {
            const ScopedLock sl(qualityInfo.lock);

            double error = (timestamp - firstTimestamp) * sampleRate - nSamples;

            if (error < -0.5) {
                qualityInfo.bufferWritePtr->dropped_super_early = 1;
                return true; // more than 50% too early, drop sample entirely
            }
            if (error > maxGapSamples) {
                LOGE("Tracker data gap of ", error, " samples is way too bad, stopping acquisition. (timestamp on latest sample is ",
                     timestamp, "; the very first timestamp was ", firstTimestamp, ").");
                return false;
            }
            if (error > 0.5) {
                // this sample is late by one or more periods: fill the gap with NaN samples so it lands within +-50%
                const int filled = static_cast<int>(std::lround(error));
                data.assign(numChannels * filled, std::nanf(""));
                sampleNumbers.resize(filled);
                timestamps.resize(filled);
                for (int i = 0; i < filled; i++) {
                    sampleNumbers[i] = nSamples + i;
                    timestamps[i] = (nSamples + i) / sampleRate;
                    qualityInfo.bufferWritePtr->filled_too_late = 1;
                    qualityInfo.stepWrite(sl);
                }
                flush(filled);
                error -= filled;
            }

            qualityInfo.bufferWritePtr->used_value = 1;
            qualityInfo.bufferWritePtr->error_is_negative = error < 0;
            qualityInfo.bufferWritePtr->error_size = (error < -0.1 || error > 0.1) + (error < -0.25 || error > 0.25);
            qualityInfo.stepWrite(sl);
        }

        // see note in readme about float32 timestamps
        const double relative = timestamp - firstTimestamp;
        data.resize(numChannels);
        data[0] = static_cast<float>(relative);
        std::copy(values, values + numChannels - 1, data.begin() + 1);
        sampleNumbers.assign(1, nSamples);
        timestamps.assign(1, relative);
        flush(1);
        return true;
    }

    void SampleWriter::flush(int numItems)
    {
        eventCodes.assign(numItems, 0);
        dataBuffer->addToBuffer(data.data(), sampleNumbers.data(), timestamps.data(), eventCodes.data(), numItems);
        nSamples += numItems;
    }

}
