#ifndef SAMPLEWRITER_H
#define SAMPLEWRITER_H

#include <DataThreadHeaders.h>
#include "QualityInfo.h"

namespace CameraTracker {

    /*
      Writes timestamped samples into the DataBuffer, "smoothing" them onto the nominal sample rate:
      samples that arrive >50% of a period early are dropped, and gaps of >50% of a period are filled
      with empty samples (see README and QualityInfo.h).

      The stream starts when acquisition does: until the first frame arrives, fillToNow() emits empty
      samples by the host clock. The first frame's camera timestamp is then anchored to its arrival time,
      and the camera clock is used from there on. Channel 0 is seconds since acquisition started (every
      sample, empty or not, has one), followed by numValues value channels.

      Only to be called from a single thread (the DataThread).
    */
    class SampleWriter {
    public:
        /** sampleRate is exact (e.g. 29.97); qualityInfo's integer rate is only for display. emptyValues
            (numValues long) is what an empty sample contains, e.g. NaN coordinates and zero areas. */
        SampleWriter(DataBuffer* dataBuffer, int numValues, double sampleRate, std::vector<float> emptyValues, QualityInfo& qualityInfo);

        /** Returns false if the stream has become unusable (huge gap) and acquisition should stop. */
        bool write(double cameraTimestamp, const float* values);

        /** Before the first frame, emits empty samples up to the current time. Call regularly. */
        void fillToNow();

    private:
        /** Appends `count` empty samples, marking them as gap-filled in the quality info. */
        void writeEmpty(int count, const ScopedLock&);

        /** Appends numItems samples from the scratch buffers (channel-major, as DataBuffer expects). */
        void flush(int numItems);

        DataBuffer* dataBuffer;
        const int numChannels;
        const double sampleRate;
        const std::vector<float> emptyValues;
        QualityInfo& qualityInfo;

        const double startMs; // host clock when acquisition started
        bool anchored = false;
        double cameraZero = 0; // camera timestamp corresponding to stream time 0

        int64 nSamples = 0;

        std::vector<float> data;
        std::vector<int64> sampleNumbers;
        std::vector<double> timestamps;
        std::vector<uint64> eventCodes;
    };

}

#endif
