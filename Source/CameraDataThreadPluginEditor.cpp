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

#include "CameraDataThreadPluginEditor.h"



namespace CameraTracker {
	DataThreadPluginEditor::DataThreadPluginEditor(GenericProcessor* parentNode, DataThreadPlugin* thread, QualityInfo& qualityInfo)
		: GenericEditor(parentNode), sampleQualityComponent(qualityInfo)
	{
        this->thread = thread;
        desiredWidth = 260;
		addComboBoxParameterEditor(Parameter::PROCESSOR_SCOPE, "camera", 10, 25);
		// Only one of these is shown, depending on the camera (see updateSettings).
		addTextBoxParameterEditor(Parameter::PROCESSOR_SCOPE, "serial", 10, 50);
		addTextBoxParameterEditor(Parameter::PROCESSOR_SCOPE, "camera_address", 10, 50);
		updateSettings();
		sampleQualityComponent.setBounds(175, 45, SampleQualityComponent::width, SampleQualityComponent::height);
        addAndMakeVisible(&sampleQualityComponent);

		openWindowButton.setBounds(10, 80, 100, 20);
		openWindowButton.setTooltip("Reopen the tracker window if it was closed (during acquisition; not available on macOS)");
		openWindowButton.onClick = [this] {
			if (!this->thread->openTrackerWindow()) {
				CoreServices::sendStatusMessage("Tracker window isn't available (macOS, or not acquiring)");
			}
		};
		addAndMakeVisible(&openWindowButton);

		statusLabel.setFont(FontOptions(11.0f));
		statusLabel.setBounds(170, 98, 90, 30);
		statusLabel.setMinimumHorizontalScale(0.5f);
		addAndMakeVisible(&statusLabel);
		startTimer(250);
	}


	DataThreadPluginEditor::~DataThreadPluginEditor() {

	}

	void DataThreadPluginEditor::updateSettings() {
		String kind = static_cast<CategoricalParameter*>(getProcessor()->getParameter("camera"))->getSelectedString();
		getParameterEditor("serial")->setVisible(kind == "spinnaker");
		getParameterEditor("camera_address")->setVisible(kind == "fake");
	}

	void DataThreadPluginEditor::startRecording() {
		thread->setRecording(true);
		thread->refreshMetadata();
	}

	void DataThreadPluginEditor::stopRecording() {
		thread->setRecording(false);
	}

	void DataThreadPluginEditor::timerCallback() {
		// Also synced here in case recording started before the tracker did.
		thread->setRecording(CoreServices::getRecordingStatus());
		thread->refreshMetadata();
		// A frame rate changed in the tracker window becomes the stream's sample rate once acquisition stops.
		if (!CoreServices::getAcquisitionStatus() && thread->isStreamRateStale()) {
			CoreServices::updateSignalChain(this);
		}
		openWindowButton.setEnabled(CoreServices::getAcquisitionStatus());
		String status = thread->getTrackerStatus();
		statusLabel.setText(status, dontSendNotification);
		statusLabel.setTooltip(status);
	}


    SampleQualityComponent::SampleQualityComponent(QualityInfo& qualityInfo_):
        qualityInfo(qualityInfo_) {
            startTimer(5);
    }

    void SampleQualityComponent::timerCallback(){
         repaint();
    }

    void SampleQualityComponent::paint(Graphics& g){

        constexpr int margin = 1;
        constexpr int nSeconds = QualityInfo::bufferSeconds;
        constexpr int colWidth = (width - margin * 2) / nSeconds;

        RectangleList<int> rectsGreen{};
        RectangleList<int> rectsRed{};
        RectangleList<int> rectsWhite{};

        {
            const ScopedLock sl(qualityInfo.lock); // when reading from qualityInfo, need to lock it

            if(qualityInfo.enabled){

                int currentBlock = qualityInfo.bufferStartIdx / qualityInfo.sampleRate;
                int cellHeight = std::max(1, (height - margin*2) / qualityInfo.sampleRate);

                for(int i=0; i<qualityInfo.buffer.size(); i++){
                    int block = i / qualityInfo.sampleRate;
                    int row = i % qualityInfo.sampleRate;

                    if(block == currentBlock && i >= qualityInfo.bufferStartIdx){
                        continue; // ignore the tail end of the buffer
                    }

                    auto v = qualityInfo.buffer[i];


                    Rectangle<int> rect{
                        margin + (block == currentBlock ? nSeconds -1 : (nSeconds -1 + block - currentBlock) % nSeconds) * colWidth,
                        height - margin - (row + 1) * cellHeight,
                        colWidth,
                        cellHeight};

                    if(v.dropped_super_early){
                        Rectangle<int> rect2 = rect;
                        rect2.setWidth(1);
                        rectsRed.addWithoutMerging(rect2);
                    }

                    if(v.filled_too_late){
                        rectsWhite.addWithoutMerging(rect);
                    } else if(v.used_value){
                        int reducedWidth = colWidth / 4 * v.error_size;
                        rect.setWidth(colWidth - reducedWidth);
                        rect.setX(rect.getX() + v.error_is_negative * reducedWidth);
                        rectsGreen.addWithoutMerging(rect);
                    }
                }
            }

        }

        g.fillAll(Colours::grey);

        g.setColour(Colours::green);
        g.fillRectList(rectsGreen);

        g.setColour(Colours::white);
        g.fillRectList(rectsWhite);

        g.setColour(Colours::red);
        g.fillRectList(rectsRed);
        //g.drawText(String(sampleRate) + " "  + String(bufferIndex), 10, 20, getWidth(), 20, Justification::centred);

    }
}