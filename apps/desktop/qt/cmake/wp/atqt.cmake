# -- WP ATQT -- Tools > Measure…'s "Several units" tab (PLAN-AT 1.4 to 1.6):
# an alignment group across units of our hub, each unit judged by its own
# connection, over the core's dspi_measure_system_*; and the S/PDIF transport
# in Window > System's "Clock and audio links" (SystemController's spdif rows).
list(APPEND DSPI_APP_SOURCES
    src/controllers/UnitsMeasureController.h src/controllers/UnitsMeasureController.cpp)
list(APPEND DSPI_QML_FILES
    qml/measure/MeasureUnitsTab.qml
    qml/system/SpdifUnitsCard.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeMeasureUnits.cpp)
