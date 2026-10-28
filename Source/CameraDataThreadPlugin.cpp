/*
 ------------------------------------------------------------------

 This file is part of the Open Ephys GUI
 Copyright (C) 2022 Open Ephys

 ------------------------------------------------------------------

 This program is free software: you can redistribute it and/or modify
 it under the terms of the GNU General Public License as published by
 the Free Software Foundation, either version 3 of the License, or
 (at your option) any later version.

 This program is distributed in the hope that it will be useful,
 but WITHOUT ANY WARRANTY; without even the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 GNU General Public License for more details.

 You should have received a copy of the GNU General Public License
 along with this program.  If not, see <http://www.gnu.org/licenses/>.

 */

#include "CameraDataThreadPlugin.h"

#include "CameraDataThreadPluginEditor.h"

#include <ProcessorHeaders.h>


namespace CameraTracker {

    // timestamp channel, then x1, y1, x2, y2, area1, area2 (see tracker/include/tracker.h)
    constexpr int NUM_VALUES = 6;
    constexpr int NUM_CHANNELS = NUM_VALUES + 1;
    constexpr int POLL_MAX_SAMPLES = 64;
    constexpr uint32_t POLL_TIMEOUT_MS = 20;
    constexpr double DEFAULT_FRAME_RATE = 50; // until the tracker window sets one

    const Array<String> CAMERA_KINDS = { "spinnaker", "fake", "synthetic" };


    DataThreadPlugin::DataThreadPlugin(SourceNode* sn) : DataThread(sn), sourceNode(sn)
    {
    }


    DataThreadPlugin::~DataThreadPlugin()
    {
        tracker_stop(tracker);
    }

    void DataThreadPlugin::registerParameters()
    {
        addCategoricalParameter(Parameter::PROCESSOR_SCOPE, "camera", "Camera", "Camera source: spinnaker = FLIR camera via the Spinnaker SDK; fake = tools/fake_camera.py over TCP", CAMERA_KINDS, 0, true);
        addStringParameter(Parameter::PROCESSOR_SCOPE, "serial", "Serial", "Spinnaker camera serial number (blank for the first camera found)", "", true);
        addStringParameter(Parameter::PROCESSOR_SCOPE, "camera_address", "Address", "host:port of the fake camera", "127.0.0.1:27030", true);
        // Not shown in the editor: frame rate, crop, rulers etc. as set in the tracker window, saved with the signal chain.
        addStringParameter(Parameter::PROCESSOR_SCOPE, "tracker_state", "Tracker state", "Settings from the tracker window (JSON)", "", false);
    }
    
    void DataThreadPlugin::parameterValueChanged (Parameter* parameter)
    {
        if (parameter->getName() == "tracker_state") {
            return; // only read when acquisition starts
        }
        CoreServices::updateSignalChain(sourceNode);
    }

    double DataThreadPlugin::getFrameRate()
    {
        var state = JSON::parse(getParameter("tracker_state")->getValueAsString());
        double fps = state["camera"]["frame_rate"];
        return fps > 0 ? fps : DEFAULT_FRAME_RATE;
    }

    bool DataThreadPlugin::isStreamRateStale()
    {
        return getFrameRate() != declaredFrameRate;
    }

    String DataThreadPlugin::buildTrackerConfig()
    {
        auto* camera = new DynamicObject();
        String kind = static_cast<CategoricalParameter*>(getParameter("camera"))->getSelectedString();
        camera->setProperty("kind", kind);
        if (kind == "spinnaker") {
            String serial = getParameter("serial")->getValueAsString().trim();
            if (serial.isNotEmpty()) {
                camera->setProperty("serial", serial);
            }
        } else if (kind == "fake") {
            camera->setProperty("address", getParameter("camera_address")->getValueAsString());
        } else {
            camera->setProperty("width", 1024);
            camera->setProperty("height", 768);
        }

        auto* config = new DynamicObject();
        config->setProperty("camera", var(camera));
        config->setProperty("show_window", true);
        var state = JSON::parse(getParameter("tracker_state")->getValueAsString());
        if (state.isObject()) {
            config->setProperty("state", state);
        }
        return JSON::toString(var(config), true);
    }


    bool DataThreadPlugin::updateBuffer()
    {
        // Called in a loop by DataThread::run(); tracker_poll blocks briefly so this doesn't spin.
        TrackerSample samples[POLL_MAX_SAMPLES];
        int64_t n = tracker_poll(tracker, samples, POLL_MAX_SAMPLES, POLL_TIMEOUT_MS);
        if (n < 0) {
            LOGE("Tracker capture thread stopped unexpectedly.");
            return false;
        }
        for (int64_t i = 0; i < n; i++) {
            if (!sampleWriter->write(samples[i].timestamp, samples[i].values)) {
                return false;
            }
        }
        return true;
    }

    bool DataThreadPlugin::foundInputSource()
    {
        return true;
    }

    bool DataThreadPlugin::startAcquisition()
    {
        if (tracker) {
            LOGE("Tracker should not be running yet when calling startAcquisition(), but it is.");
            return false;
        }
        if (!sourceBuffers.getFirst()) {
            LOGE("sourceBuffer should be allocated when calling startAcquisition(), but it isn't.");
            return false;
        }
        sourceBuffers.getFirst()->clear();

        // The stream's sample rate was declared in updateSettings(); a newer frame rate from the window waits
        // for the next signal chain update.
        qualityInfo.initialise(std::max(1, roundToInt(declaredFrameRate)), true);
        sampleWriter = std::make_unique<SampleWriter>(sourceBuffers.getFirst(), NUM_VALUES, declaredFrameRate, qualityInfo);

        String config = buildTrackerConfig();
        LOGC("Starting tracker with config: ", config);
        char error[512] = {};
        tracker = tracker_start(config.toRawUTF8(), error, sizeof(error));
        if (!tracker) {
            LOGE("Failed to start tracker: ", error);
            CoreServices::sendStatusMessage("Tracker failed to start!");
            return false;
        }

        startThread(); // DataThread::run() calls updateBuffer() until stopped
        return true;
    }

    bool DataThreadPlugin::stopAcquisition()
    {
        if (!tracker) {
            LOGE("Tracker should be running when calling stopAcquisition(), but it isn't.");
            return false;
        }
        if (!stopThread(500)) {
            return false;
        }
        saveTrackerState();
        tracker_stop(tracker);
        tracker = nullptr;
        sampleWriter = nullptr;
        sourceBuffers.getFirst()->clear();
        return true;
    }

    String DataThreadPlugin::currentTrackerState()
    {
        if (!tracker) {
            return getParameter("tracker_state")->getValueAsString();
        }
        std::vector<char> buf(4096);
        size_t len = tracker_state_json(tracker, buf.data(), buf.size());
        if (len >= buf.size()) {
            buf.resize(len + 1);
            len = tracker_state_json(tracker, buf.data(), buf.size());
        }
        return String::fromUTF8(buf.data(), (int) len);
    }

    void DataThreadPlugin::saveTrackerState()
    {
        String state = currentTrackerState();
        if (state.isNotEmpty()) {
            getParameter("tracker_state")->setNextValue(state, false);
        }
    }

    void DataThreadPlugin::refreshMetadata()
    {
        if (!metadataPixelsPerMetre) {
            return; // updateSettings hasn't run yet
        }
        TrackerMetadata m;
        if (!tracker_metadata(currentTrackerState().toRawUTF8(), &m)) {
            return;
        }
        // Downstream copies of the stream share these value objects, so this updates what record engines
        // see too. Settings are locked while recording, so the values are stable for a recording.
        // Explicit template arguments pick the array overloads that Open Ephys exports (setValue<T>(const T*)).
        metadataPixelsPerMetre->setValue<double>(m.pixels_per_metre);
        metadataWindow->setValue<int32>(static_cast<const int32*>(m.window));
        metadataCrop->setValue<double>(static_cast<const double*>(m.crop));
        metadataFrameSize->setValue<int32>(static_cast<const int32*>(m.frame_size));
    }

    void DataThreadPlugin::setRecording(bool recording)
    {
        if (tracker) {
            tracker_set_recording(tracker, recording);
        }
    }

    bool DataThreadPlugin::openTrackerWindow()
    {
        return tracker && tracker_open_window(tracker);
    }

    String DataThreadPlugin::getTrackerStatus()
    {
        if (!tracker) {
            return {};
        }
        char status[256] = {};
        tracker_status(tracker, status, sizeof(status));
        return String::fromUTF8(status);
    }


    void DataThreadPlugin::updateSettings(OwnedArray<ContinuousChannel>* continuousChannels,
        OwnedArray<EventChannel>* eventChannels,
        OwnedArray<SpikeChannel>* spikeChannels,
        OwnedArray<DataStream>* sourceStreams,
        OwnedArray<DeviceInfo>* devices,
        OwnedArray<ConfigurationObject>* configurationObjects)
    {
        if (tracker) {
            LOGE("Tracker should not be running when updating Settings, but it is.");
            return;
        }

        sourceStreams->clear();
        sourceBuffers.clear();
        continuousChannels->clear();

        declaredFrameRate = getFrameRate();
        // Stream, channel and event identifiers keep their original "bonsai"/"BONS" names so existing
        // recordings and downstream readers stay compatible.
        DataStream* stream = new DataStream({
           "bonsai",
           "LED positions from the camera tracker",
           "bonsai",
           static_cast<float>(declaredFrameRate),
        });

        sourceStreams->add(stream);

        sourceBuffers.add(new DataBuffer(NUM_CHANNELS, 1024));

        for (int i = 0; i < NUM_CHANNELS; i++) {
            continuousChannels->add(new ContinuousChannel({
                ContinuousChannel::Type::AUX,
                "BONS" + std::to_string(i+1),
                i == 0 ? "float32 camera timestamp (s, relative to first frame)" : "float32 value from tracker",
                "BONS" + std::to_string(i+1),
                6.,  // channel bitvolts scaling, not especially relevant here
                stream
             }));
        }


        // Tracker settings for record engines (e.g. for file headers); see README. Identifiers are stable.
        struct Field { MetadataValuePtr* value; MetadataDescriptor::MetadataType type; unsigned int length; const char* name; const char* description; };
        const Field fields[] = {
            { &metadataPixelsPerMetre, MetadataDescriptor::DOUBLE, 1, "pixels_per_metre", "Mean of the calibration rulers; NaN if not calibrated" },
            { &metadataWindow, MetadataDescriptor::INT32, 4, "window", "Crop bounding box in frame pixels, clipped to the frame: min_x, min_y, max_x, max_y" },
            { &metadataCrop, MetadataDescriptor::DOUBLE, 6, "crop", "Crop: shape (0 none, 1 circle, 2 square, 3 rectangle), centre x, centre y, width, height, angle (degrees, clockwise)" },
            { &metadataFrameSize, MetadataDescriptor::INT32, 2, "frame_size", "Camera frame width and height in pixels; 0 if not known yet" },
        };
        for (const Field& f : fields) {
            MetadataDescriptor descriptor(f.type, f.length, f.name, f.description, String("camera_tracker.") + f.name);
            *f.value = new MetadataValue(descriptor);
            stream->addMetadata(new MetadataDescriptor(descriptor), f.value->get());
            // Also on the first channel, for record engines that work from channels rather than streams.
            continuousChannels->getFirst()->addMetadata(new MetadataDescriptor(descriptor), f.value->get());
        }
        refreshMetadata();

        // we don't currently use the event channel for anything but the RecordEngine requires one (or it crashes on startup)
        eventChannels->add(new EventChannel({
            EventChannel::Type::TTL,
            "Camera Event Channel",
            "Dummy channel, required for RecordEngine to not crash",
            "bonsai_ttl_events",
            stream,
            1
         }));
    }


    void DataThreadPlugin::resizeBuffers()
    {

    }


    std::unique_ptr<GenericEditor> DataThreadPlugin::createEditor(SourceNode* sn)
    {
        std::unique_ptr<DataThreadPluginEditor> editor = std::make_unique<DataThreadPluginEditor>(sn, this, qualityInfo);
        return editor;

    }


    void DataThreadPlugin::handleBroadcastMessage (const String& msg, const int64 messageTimestmpMilliseconds)
    {
    }

    String DataThreadPlugin::handleConfigMessage (const String& msg)
    {
        return "";
    }


}
