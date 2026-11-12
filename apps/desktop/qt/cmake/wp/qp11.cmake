# -- WP QP11 -- Hardware settings pages: Inputs, Outputs, I2S Configuration,
# Global Parameters, Control Interfaces, and the save bar
list(APPEND DSPI_APP_SOURCES
    src/controllers/HardwareController.h src/controllers/HardwareController.cpp
    src/controllers/NamesController.h src/controllers/NamesController.cpp
    src/controllers/OutputLatencyController.h src/controllers/OutputLatencyController.cpp)
list(APPEND DSPI_QML_FILES
    qml/settings/SettingsSaveBar.qml
    qml/settings/hardware/HwRow.qml
    qml/settings/hardware/ChoicePicker.qml
    qml/settings/hardware/PinPicker.qml
    qml/settings/hardware/StatusLine.qml
    qml/settings/hardware/NameField.qml
    qml/settings/pages/InputsPage.qml
    qml/settings/pages/OutputsPage.qml
    qml/settings/pages/I2sPage.qml
    qml/settings/pages/GlobalPage.qml
    qml/settings/pages/InterfacesPage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeHardware.cpp)
