# -- WP PGQT -- PLAN-PG on the desktop: the open device's protocol in Settings >
# About, the raw request by command id, the alink sender's page, and the smoke
# sections on each fake generation (the update screen's switch to the core's
# update check is in FirmwareController, QP16's files)
list(APPEND DSPI_APP_SOURCES
    src/controllers/DeviceInfoController.h src/controllers/DeviceInfoController.cpp)
list(APPEND DSPI_QML_FILES
    qml/deviceinfo/DeviceInfoRows.qml
    qml/deviceinfo/RawRequestDialog.qml
    qml/deviceinfo/SenderPage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeGenerations.cpp)
