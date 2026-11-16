# -- WP P5QCAP -- The microphone (PLAN-P5 OD-P5s): Qt Multimedia's QAudioSource
# read off the GUI thread (src/platform/AudioCapture), the P5SPIKE development
# switches (src/platform/MeasureSpike), and the capture smoke test fed from a
# WAV file through the reader's test seam.
list(APPEND DSPI_APP_SOURCES
    src/platform/AudioCapture.h src/platform/AudioCapture.cpp
    src/platform/MeasureSpike.h src/platform/MeasureSpike.cpp)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeCapture.cpp)
list(APPEND DSPI_QT_COMPONENTS Multimedia)
list(APPEND DSPI_APP_LIBS Qt6::Multimedia)

# Nothing is ever played by the host (PLAN-P5 S1, OD-P5a): the stimulus is the
# DSPi's generator. No source file of the app or its QML may name a Qt audio or
# media output class or import the QtMultimedia QML module (its MediaPlayer,
# SoundEffect, AudioOutput); the build stops here if one does.
file(GLOB_RECURSE p5qcap_files CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.h" "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/qml/*.qml" "${CMAKE_CURRENT_SOURCE_DIR}/smoke/*.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/smoke/*.h" "${CMAKE_CURRENT_SOURCE_DIR}/smoke/*.qml"
    # -- Review P5-B -- every other kind of source the app could grow: a QML
    # script's `.import QtMultimedia`, Objective-C++, other C++ spellings
    "${CMAKE_CURRENT_SOURCE_DIR}/qml/*.js" "${CMAKE_CURRENT_SOURCE_DIR}/qml/*.mjs"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.mm" "${CMAKE_CURRENT_SOURCE_DIR}/src/*.hpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cc" "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cxx"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.qml" "${CMAKE_CURRENT_SOURCE_DIR}/src/*.js")
foreach(f IN LISTS p5qcap_files)
    file(STRINGS "${f}" p5qcap_hits REGEX "QAudioSink|QAudioOutput|QMediaPlayer|QSoundEffect|QAudioEngine|QSpatialSound|QAmbientSound|import[ \t]+QtMultimedia")
    if(p5qcap_hits)
        message(FATAL_ERROR "${f} plays audio or imports QtMultimedia's QML (${p5qcap_hits}): the host never plays anything (PLAN-P5 S1)")
    endif()
endforeach()
