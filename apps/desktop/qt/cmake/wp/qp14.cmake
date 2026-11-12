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
# Windows' IP helper and shell. The targets are made after this file is read, so
# the libraries are added once they exist.
find_package(Qt6 6.8 REQUIRED COMPONENTS Network)
function(dspi_qp14_link)
    foreach(target DSPiConsole DSPiConsoleSmoke)
        if(TARGET ${target})
            target_link_libraries(${target} PRIVATE Qt6::Network)
            if(WIN32)
                target_link_libraries(${target} PRIVATE iphlpapi shell32)
            endif()
        endif()
    endforeach()
endfunction()
cmake_language(DEFER CALL dspi_qp14_link)
