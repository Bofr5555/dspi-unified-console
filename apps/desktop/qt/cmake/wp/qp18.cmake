# -- WP QP18 -- Polish across the tree: undo, redo and the change history;
# notices instead of success alerts; the keyboard rows (presets 1-9, mute,
# bypass); the demo banner; the Mode entry; the host's audio format; the Pro
# layer's components (LevelMeter, StatusBar, NumericField)
list(APPEND DSPI_APP_SOURCES
    src/controllers/HistoryController.h src/controllers/HistoryController.cpp
    src/controllers/NoticeCenter.h src/controllers/NoticeCenter.cpp
    src/controllers/ShortcutController.h src/controllers/ShortcutController.cpp
    src/controllers/DemoController.h src/controllers/DemoController.cpp
    src/controllers/ModeController.h src/controllers/ModeController.cpp
    src/platform/HostAudioFormat.h src/platform/HostAudioFormat.cpp
    src/platform/HostAudioFormatLinux.cpp src/platform/HostAudioFormatWindows.cpp)
list(APPEND DSPI_QML_FILES
    qml/components/LevelMeter.qml
    qml/components/StatusBar.qml
    qml/components/NumericField.qml
    qml/banners/NoticeBanner.qml
    qml/banners/DemoBanner.qml
    qml/main/ModeWindow.qml
    qml/tools/HistoryWindow.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/Qp18Notices.h smoke/SmokePolish.cpp smoke/SmokeAccess.cpp)

# The Windows host format (WASAPI, the device tree)
if(WIN32)
    list(APPEND DSPI_APP_LIBS ole32 cfgmgr32)
endif()
