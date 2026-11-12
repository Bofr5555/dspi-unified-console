#ifndef SMOKEPROBE_H
#define SMOKEPROBE_H

#include "Registrar.h"

#include <QObject>
#include <QtQml/qqmlregistration.h>

class QQmlEngine;
class QJSEngine;

// -- WP QP0 -- A controller registered the way a work package registers one
// (Registrar::Controller), with a banner and a tool window, so the smoke
// driver proves each plug-in point works. Only in uconsole-for-dspi-smoke.
class SmokeProbe : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString message READ message NOTIFY messageChanged)
    Q_PROPERTY(bool bannerShown READ bannerShown WRITE setBannerShown NOTIFY messageChanged)
public:
    SmokeProbe(const AppContext &ctx, QObject *parent);
    static SmokeProbe *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<SmokeProbe>(); }

    QString message() const { return m_message; }
    bool bannerShown() const { return m_bannerShown; }
    void setBannerShown(bool shown);
    int actions() const { return m_actions; }

signals:
    void messageChanged();

private:
    QString m_message;
    bool m_bannerShown = false;
    int m_actions = 0;
};

#endif // SMOKEPROBE_H
