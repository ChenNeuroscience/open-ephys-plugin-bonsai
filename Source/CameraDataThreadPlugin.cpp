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

    const Array<String> CAMERA_KINDS = { "fake", "synthetic" };


    DataThreadPlugin::DataThreadPlugin(SourceNode* sn) : DataThread(sn), sourceNode(sn)
    {
    }


    DataThreadPlugin::~DataThreadPlugin()
    {
        tracker_stop(tracker);
    }

    void DataThreadPlugin::registerParameters()
    {
        addCategoricalParameter(Parameter::PROCESSOR_SCOPE, "camera", "Camera", "Camera source (fake = tools/fake_camera.py over TCP)", CAMERA_KINDS, 0, true);
        addStringParameter(Parameter::PROCESSOR_SCOPE, "camera_address", "Address", "host:port of the fake camera", "127.0.0.1:27030", true);
        addBooleanParameter(Parameter::PROCESSOR_SCOPE, "show_window", "Window", "Open the tracker window during acquisition (not supported on macOS)", true, true);
        addIntParameter(Parameter::PROCESSOR_SCOPE, "sample_rate","Sample Rate", "Sample Rate (Hz) to show on data stream; should match the camera frame rate.", 50, 1, 1000, true);
        // Not shown in the editor: crop, rulers, threshold etc. as set in the tracker window, saved with the signal chain.
        addStringParameter(Parameter::PROCESSOR_SCOPE, "tracker_state", "Tracker state", "Settings from the tracker window (JSON)", "", false);
    }
    
    void DataThreadPlugin::parameterValueChanged (Parameter* parameter)
    {
        if (parameter->getName() == "tracker_state") {
            return; // only read when acquisition starts
        }
        CoreServices::updateSignalChain(sourceNode);
    }

    String DataThreadPlugin::buildTrackerConfig()
    {
        auto* camera = new DynamicObject();
        String kind = static_cast<CategoricalParameter*>(getParameter("camera"))->getSelectedString();
        camera->setProperty("kind", kind);
        if (kind == "fake") {
            camera->setProperty("address", getParameter("camera_address")->getValueAsString());
        } else {
            camera->setProperty("width", 1024);
            camera->setProperty("height", 768);
            camera->setProperty("fps", (int) getParameter("sample_rate")->getValue());
        }

        auto* config = new DynamicObject();
        config->setProperty("camera", var(camera));
        config->setProperty("show_window", (bool) getParameter("show_window")->getValue());
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

        qualityInfo.initialise(getParameter("sample_rate")->getValue(), true);
        sampleWriter = std::make_unique<SampleWriter>(sourceBuffers.getFirst(), NUM_VALUES, qualityInfo);

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

    void DataThreadPlugin::saveTrackerState()
    {
        std::vector<char> buf(4096);
        size_t len = tracker_state_json(tracker, buf.data(), buf.size());
        if (len >= buf.size()) {
            buf.resize(len + 1);
            len = tracker_state_json(tracker, buf.data(), buf.size());
        }
        if (len > 0) {
            getParameter("tracker_state")->setNextValue(String::fromUTF8(buf.data(), (int) len), false);
        }
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

        int sampleRate = getParameter("sample_rate")->getValue();
        // Stream, channel and event identifiers keep their original "bonsai"/"BONS" names so existing
        // recordings and downstream readers stay compatible.
        DataStream* stream = new DataStream({
           "bonsai",
           "LED positions from the camera tracker",
           "bonsai",
           static_cast<float>(sampleRate),
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
