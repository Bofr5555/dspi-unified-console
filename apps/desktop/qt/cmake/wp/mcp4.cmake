# -- WP MCP4 -- AI agents: Settings > AI Agents, what they ask the user, what they
# did, and Classic's window and status icon
list(APPEND DSPI_APP_SOURCES
    src/controllers/AiController.h src/controllers/AiController.cpp)
list(APPEND DSPI_QML_FILES
    qml/ai/AiAgentsPage.qml qml/ai/AiAgentsPanel.qml qml/ai/AiActivityList.qml
    qml/ai/AiHost.qml qml/ai/AiSheet.qml qml/ai/AiAnswerButtons.qml
    qml/ai/AiStatusIcon.qml qml/ai/AiCopyField.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeAi.cpp)
