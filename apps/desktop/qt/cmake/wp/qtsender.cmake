# -- WP QTSENDER -- The alink sender: one banner in the main window, the DSP
# menus, shortcuts and tool windows greyed (DeviceInfoController.senderKeeps,
# Shell's action gate), and the smoke section on the legacy and the
# dual-stack sender
list(APPEND DSPI_QML_FILES qml/banners/SenderBanner.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeQtSender.cpp)
