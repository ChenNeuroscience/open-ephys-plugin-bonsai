#ifndef SAMPLEWRITER_H
#define SAMPLEWRITER_H

#include <DataThreadHeaders.h>
#include "QualityInfo.h"

namespace CameraTracker {

    /*
      Writes timestamped samples into the DataBuffer, "smoothing" them onto the nominal sample rate:
      samples that arrive >50% of a period early are dropped, and gaps of >50% of a period are filled
      with NaN samples (see README and QualityInfo.h). Channel 0 is the timestamp relative to the first
      sample, followed by numValues value channels.

      Only to be called from a single thread (the DataThread).
    */
    class SampleWriter {
    public:
        /** sampleRate is exact (e.g. 29.97); qualityInfo's integer rate is only for display. */
        SampleWriter(DataBuffer* dataBuffer, int numValues, double sampleRate, QualityInfo& qualityInfo);

        /** Returns false if the stream has become unusable (huge gap) and acquisition should stop. */
        bool write(double timestamp, const float* values);

    private:
        /** Appends numItems samples from the scratch buffers (channel-major, as DataBuffer expects). */
        void flush(int numItems);

        DataBuffer* dataBuffer;
        const int numChannels;
        const double sampleRate;
        QualityInfo& qualityInfo;

        int64 nSamples = 0;
        double firstTimestamp = 0;

        std::vector<float> data;
        std::vector<int64> sampleNumbers;
        std::vector<double> timestamps;
        std::vector<uint64> eventCodes;
    };

}

#endif
