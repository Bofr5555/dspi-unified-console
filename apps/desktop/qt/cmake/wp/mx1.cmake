# -- WP MX1 -- The matrix mixer redrawn (DESREV's matrix-mockup.html): the
# column and row headers and the tools as their own files; the keyboard, Link
# L/R pairs and both densities in the smoke section.
list(APPEND DSPI_QML_FILES
    qml/tools/MatrixColumnHeader.qml
    qml/tools/MatrixRowHeader.qml
    qml/tools/MatrixTools.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeMx1.cpp)
