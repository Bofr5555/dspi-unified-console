# -- WP UA2 -- (A2, QT-MEASUREBAR) Measure's action bar and a refused Measure's
# rows; the smoke section ua2-measurebar
list(APPEND DSPI_QML_FILES
    qml/measure/MeasureActionBar.qml
    qml/measure/MeasureRefusalRow.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeUa2.cpp)
