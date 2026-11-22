# -- WP QP8 -- Effects B: the subharmonic synthesizer, the tube modeller, the upmixer, the limiter's settings
list(APPEND DSPI_APP_SOURCES
    src/controllers/ToolsController.h src/controllers/ToolsController.cpp
    src/render/TubeIllustration.h src/render/TubeIllustration.cpp)
list(APPEND DSPI_QML_FILES
    qml/tools/ToolHeader.qml qml/tools/ToolParameterRow.qml qml/tools/ToolSegments.qml qml/tools/ToolUnsupported.qml
    qml/tools/SubharmonicGraph.qml qml/tools/TubeTransferGraph.qml
    qml/tools/LimiterSettings.qml)   # -- WP LAY1a -- the windows are panels (lay1a.cmake)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeTools.cpp)
