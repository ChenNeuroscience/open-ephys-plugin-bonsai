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

#ifndef DATATHREADPLUGINEDITOR_H_DEFINED
#define DATATHREADPLUGINEDITOR_H_DEFINED

#include <EditorHeaders.h>
#include <ProcessorHeaders.h>
#include "QualityInfo.h"
#include "CameraDataThreadPlugin.h"

namespace CameraTracker {


    class SampleQualityComponent : public Component, Timer {
    public:
        SampleQualityComponent(QualityInfo& qualityInfo_);
        ~SampleQualityComponent() {};

        QualityInfo& qualityInfo;

        void timerCallback() override;
        void paint(Graphics& g) override;

        constexpr static int width = 82;
        constexpr static int height = 50;
    private:
        /** Generates an assertion if this class leaks */
        JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR(SampleQualityComponent);
    };





	class DataThreadPluginEditor : public GenericEditor, Timer
	{
	public:
		/** The class constructor, used to initialize any members. */
		DataThreadPluginEditor (GenericProcessor* parentNode, DataThreadPlugin* thread, QualityInfo& qualityInfo);

		/** The class destructor, used to deallocate memory */
		~DataThreadPluginEditor();

		/** Shows the camera fields relevant to the selected camera */
		void updateSettings() override;

		/** Refreshes the tracker status line */
		void timerCallback() override;

	private:

		/** A pointer to the underlying DataThreadPlugin */
    	DataThreadPlugin* thread;

        SampleQualityComponent sampleQualityComponent;

        Label statusLabel;

    };

}
#endif