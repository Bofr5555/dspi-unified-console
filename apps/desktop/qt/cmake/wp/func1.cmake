# -- WP FUNC1 -- The function test's way to act as the user (tools/functest):
# a local socket that reads and calls the app's own controllers. Off in every
# build but the function test's, and inert there without DSPI_FUNCTEST_SOCKET.
option(DSPI_FUNCTEST "Build the function test's control socket into the app (never in a release)" OFF)
if(DSPI_FUNCTEST)
    list(APPEND DSPI_APP_SOURCES src/functest/FuncTest.h src/functest/FuncTest.cpp)
    list(APPEND DSPI_QT_COMPONENTS Network)
    list(APPEND DSPI_APP_LIBS Qt6::Network)
endif()
