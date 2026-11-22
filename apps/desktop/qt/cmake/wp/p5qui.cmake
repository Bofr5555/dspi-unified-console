# -- WP P5QUI -- Tools > Measure… (PLAN-P5 P5QUI): the measurement, the live
# view, the auto-tune and the documents over the core's dspi_measure_* calls,
# with P5QCAP's AudioCapture (capture only) or the demo's simulated microphone.
list(APPEND DSPI_APP_SOURCES
    src/controllers/MeasureController.h src/controllers/MeasureController.cpp
    src/controllers/TuneController.h src/controllers/TuneController.cpp)
list(APPEND DSPI_QML_FILES
    qml/measure/MeasurePanel.qml   # -- WP LAY1b -- was MeasureWindow.qml
    qml/measure/MeasureGraph.qml
    qml/measure/MeasureSetupTab.qml
    qml/measure/MeasureResultsTab.qml
    qml/measure/MeasureLiveTab.qml
    qml/measure/MeasureTuneTab.qml
    qml/measure/MeasureDocsTab.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeMeasure.cpp)
