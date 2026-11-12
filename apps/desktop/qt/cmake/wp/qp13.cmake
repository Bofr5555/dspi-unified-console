# -- WP QP13 -- Devices and Link: the device list, the pairing prompt, the Link
# client's controller and its banners (TokenStore is the base's list already)
list(APPEND DSPI_APP_SOURCES
    src/controllers/LinkController.h src/controllers/LinkController.cpp
    src/controllers/DevicesController.h src/controllers/DevicesController.cpp)
list(APPEND DSPI_QML_FILES
    qml/devices/DevicesHost.qml
    qml/devices/DevicesPopup.qml
    qml/devices/DeviceRow.qml
    qml/devices/PairingDialog.qml
    qml/link/LinkBanner.qml
    qml/link/LockNotice.qml
    qml/link/TokenNotice.qml)
list(APPEND DSPI_SMOKE_SOURCES
    smoke/LinkFakeHub.h
    smoke/SmokeLink.cpp
    smoke/SmokeDevices.cpp
    smoke/SmokeLinkFault.cpp)
