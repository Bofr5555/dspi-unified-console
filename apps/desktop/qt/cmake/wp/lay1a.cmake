# -- WP LAY1a -- The matrix mixer and the seven effect windows as panels
list(APPEND DSPI_QML_FILES
    qml/tools/MatrixPanel.qml
    qml/tools/CrossfeedPanel.qml qml/tools/LoudnessPanel.qml qml/tools/LevellerPanel.qml
    qml/tools/PsybassPanel.qml qml/tools/SubharmonicPanel.qml qml/tools/TubePanel.qml
    qml/tools/UpmixerPanel.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay1a.cpp)
