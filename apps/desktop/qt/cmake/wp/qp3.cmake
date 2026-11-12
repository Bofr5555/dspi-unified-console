# -- WP QP3 -- Response graph
list(APPEND DSPI_APP_SOURCES
    src/controllers/GraphController.h src/controllers/GraphController.cpp
    src/render/BodePlot.h src/render/BodePlot.cpp)
list(APPEND DSPI_QML_FILES
    qml/main/GraphHost.qml qml/main/GraphPlot.qml qml/main/GraphLegend.qml
    qml/main/GraphOptions.qml qml/main/GraphSetupPage.qml
    qml/tools/GraphWindow.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeGraph.cpp)
