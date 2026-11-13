# -- WP QP6 -- Matrix mixer
list(APPEND DSPI_APP_SOURCES
    src/controllers/MatrixModel.h src/controllers/MatrixModel.cpp
    src/controllers/MatrixController.h src/controllers/MatrixController.cpp)
list(APPEND DSPI_QML_FILES
    qml/tools/MatrixField.qml
    qml/tools/MatrixCell.qml
    qml/tools/MatrixWindow.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeMatrix.cpp)
