#include "SampleWriter.h"

namespace CameraTracker {

    // Gaps longer than this many samples are treated as a lost source rather than filled.
    constexpr double maxGapSamples = 1000;

    SampleWriter::SampleWriter(DataBuffer* dataBuffer_, int numValues, double sampleRate_, std::vector<float> emptyValues_, QualityInfo& qualityInfo_)
        : dataBuffer(dataBuffer_), numChannels(numValues + 1), sampleRate(sampleRate_), emptyValues(std::move(emptyValues_)),
          qualityInfo(qualityInfo_), startMs(Time::getMillisecondCounterHiRes())
    {
        jassert((int) emptyValues.size() == numValues);
    }

    void SampleWriter::fillToNow()
    {
        if (anchored) {
            return; // the camera clock is in charge; gaps are filled when the next frame arrives
        }
        // Only samples at least half a period in the past, so the first frame is never "too early".
        const double elapsed = (Time::getMillisecondCounterHiRes() - startMs) / 1000.0;
        const int64 target = static_cast<int64>(std::ceil(elapsed * sampleRate - 0.5));
        if (target > nSamples) {
            const ScopedLock sl(qualityInfo.lock);
            writeEmpty(static_cast<int>(target - nSamples), sl);
        }
    }

    bool SampleWriter::write(double cameraTimestamp, const float* values)
    {
        if (!anchored) {
            // Place the first frame at the moment it arrived, relative to acquisition start.
            const double elapsed = (Time::getMillisecondCounterHiRes() - startMs) / 1000.0;
            cameraZero = cameraTimestamp - elapsed;
            anchored = true;
        }

        const ScopedLock sl(qualityInfo.lock);
        double error = (cameraTimestamp - cameraZero) * sampleRate - nSamples;

        if (error < -maxGapSamples) {
            // The camera clock went backwards (e.g. it reconnected and restarted): re-anchor it at the next sample.
            LOGC("Camera clock jumped back by ", -error / sampleRate, " s; re-anchoring it to the stream.");
            cameraZero = cameraTimestamp - nSamples / sampleRate;
            error = 0;
        }
        if (error < -0.5) {
            qualityInfo.bufferWritePtr->dropped_super_early = 1;
            return true; // more than 50% too early, drop sample entirely
        }
        if (error > maxGapSamples) {
            LOGE("Tracker data gap of ", error, " samples is way too bad, stopping acquisition. (camera timestamp on latest sample is ",
                 cameraTimestamp, "; stream time zero is camera time ", cameraZero, ").");
            return false;
        }
        if (error > 0.5) {
            // this sample is late by one or more periods: fill the gap so it lands within +-50%
            const int filled = static_cast<int>(std::lround(error));
            writeEmpty(filled, sl);
            error -= filled;
        }

        qualityInfo.bufferWritePtr->used_value = 1;
        qualityInfo.bufferWritePtr->error_is_negative = error < 0;
        qualityInfo.bufferWritePtr->error_size = (error < -0.1 || error > 0.1) + (error < -0.25 || error > 0.25);
        qualityInfo.stepWrite(sl);

        // see note in readme about float32 timestamps
        const double seconds = cameraTimestamp - cameraZero;
        data.resize(numChannels);
        data[0] = static_cast<float>(seconds);
        std::copy(values, values + numChannels - 1, data.begin() + 1);
        sampleNumbers.assign(1, nSamples);
        timestamps.assign(1, seconds);
        flush(1);
        return true;
    }

    void SampleWriter::writeEmpty(int count, const ScopedLock& sl)
    {
        // Channel-major: the timestamp channel first (nominal times), then each value channel's empty value.
        data.resize(numChannels * count);
        sampleNumbers.resize(count);
        timestamps.resize(count);
        for (int i = 0; i < count; i++) {
            sampleNumbers[i] = nSamples + i;
            timestamps[i] = (nSamples + i) / sampleRate;
            data[i] = static_cast<float>(timestamps[i]);
            qualityInfo.bufferWritePtr->filled_too_late = 1;
            qualityInfo.stepWrite(sl);
        }
        for (int c = 1; c < numChannels; c++) {
            std::fill(data.begin() + c * count, data.begin() + (c + 1) * count, emptyValues[c - 1]);
        }
        flush(count);
    }

    void SampleWriter::flush(int numItems)
    {
        eventCodes.assign(numItems, 0);
        dataBuffer->addToBuffer(data.data(), sampleNumbers.data(), timestamps.data(), eventCodes.data(), numItems);
        nSamples += numItems;
    }

}
