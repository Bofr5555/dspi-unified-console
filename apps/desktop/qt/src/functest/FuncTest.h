#ifndef FUNCTEST_H
#define FUNCTEST_H

#include <QJsonObject>
#include <QObject>

#include "Registrar.h"

class QLocalServer;

// -- WP FUNC1 -- The function test acts as the user through the app's own
// controllers (tools/functest/README.md). Compiled with DSPI_FUNCTEST only, and
// it listens only when DSPI_FUNCTEST_SOCKET names a socket. One JSON object a
// line, one answer a line:
//
//   {"op":"get","on":"AiController","prop":"confirmations"}
//   {"op":"set","on":"AiController","prop":"confirmMode","value":"ask_risky"}
//   {"op":"call","on":"AiController","method":"answer","args":[12,true]}
//   {"op":"page","section":"equaliser","tab":""}   a page of Modern
//   {"op":"grab","dir":"/out"}      every visible window, as PNG files
//   {"op":"quit"}
//
// `on` is a controller's class, or DeviceRegistry or DeviceModel.
class FuncTest : public QObject {
    Q_OBJECT
public:
    FuncTest(const AppContext &ctx, QObject *parent);

private:
    QJsonObject handle(const QJsonObject &command);
    QObject *find(const QString &name) const;

    AppContext m_ctx;
    QLocalServer *m_server = nullptr;
};

#endif
