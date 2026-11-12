# -- WP QP23 -- The Core Voltage page (behaviour.md §20): Settings > Advanced >
# Core Voltage, the core's calibration flow and manual requests
list(APPEND DSPI_APP_SOURCES
    src/controllers/CoreVoltageController.h src/controllers/CoreVoltageController.cpp)
list(APPEND DSPI_QML_FILES qml/settings/pages/CoreVoltagePage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeCoreVoltage.cpp)
