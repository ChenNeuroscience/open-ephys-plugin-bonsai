#ifndef QUALITYINFO_H
#define QUALITYINFO_H

#include <ProcessorHeaders.h>

namespace CameraTracker {
#pragma pack(push, 1)
/*
  When timestamps are provided in samples, we check the timestamp on a given sample relative to what it should be
  if we'd been seeing perfect sampling frequency relative to the first timestamp seen. If a given sample is earlier by
  over 50% of the sampling interval then we drop the sample entirely. If a sample is more than 50% late, then we fill
  the gap with one or more samples consisting of nans. Note that for a single sample it's possible that both of these
  things happened, i.e. we dropped a prior sample that was super early but then had to fill the gap. If a sample does
  arrive within +-50% of the expected time we record the error.
*/
struct SampleQuality {
    uint8_t dropped_super_early: 1;
    uint8_t filled_too_late: 1;
    uint8_t used_value: 1; // filled_too_late=1 and used_value=1 are mutually exclusive
    uint8_t error_is_negative: 1; // only set if used_value=1
    uint8_t error_size: 2;  // only set if used_value=1. within 10%: 0, within 25%: 1, within 50%: 2
};
#pragma pack(pop)


    class QualityInfo {
        public:
        CriticalSection lock;
        std::vector<SampleQuality> buffer {};
        size_t bufferStartIdx = 0;
        SampleQuality* bufferWritePtr = nullptr;
        int sampleRate = 0;
        constexpr static int bufferSeconds = 10;
        bool enabled = false;
        // TODO: should really track how long it's been since any data so we can show the gap building up

        void stepWrite(const ScopedLock&){
            // should be called while already locked, hence the required argument
            bufferWritePtr = &buffer[bufferStartIdx];
            *bufferWritePtr = {};
            bufferStartIdx = (bufferStartIdx + 1) % buffer.size();
        }

        void initialise(int sampleRate_ , bool enabled_){
            const ScopedLock sl(lock);
            sampleRate = sampleRate_;
            enabled = enabled_;
            // if not enabled this is redundant, but whatever...
            buffer.resize(bufferSeconds * sampleRate);
            std::memset(&buffer[0], 0, buffer.size() * sizeof(SampleQuality));
            bufferStartIdx = 1;
            bufferWritePtr = &buffer[0];
        }
    };

}

#endif