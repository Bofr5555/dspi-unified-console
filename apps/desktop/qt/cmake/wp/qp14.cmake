# -- WP QP14 -- Hub, networking, tray, system: Settings > Networking, the hub's
# controller, the tray, open at login and keep awake, the Windows Firewall rows
list(APPEND DSPI_APP_SOURCES
    src/controllers/HubController.h src/controllers/HubController.cpp
    src/platform/TrayController.h src/platform/TrayController.cpp
    src/platform/SystemIntegration.h src/platform/SystemIntegration.cpp
    src/platform/FirewallController.h src/platform/FirewallController.cpp)
list(APPEND DSPI_QML_FILES qml/settings/pages/NetworkingPage.qml)
list(APPEND DSPI_SMOKE_SOURCES
    smoke/SmokeNetworking.cpp
    smoke/SmokeTray.cpp
    smoke/SmokeSystem.cpp)

# The address line needs Qt Network (QNetworkInterface); the firewall rows need
# Windows' IP helper and shell (-- WP QPI2 -- through the base's lists)
list(APPEND DSPI_QT_COMPONENTS Network)
list(APPEND DSPI_APP_LIBS Qt6::Network)
if(WIN32)
    list(APPEND DSPI_APP_LIBS iphlpapi shell32)
endif()
