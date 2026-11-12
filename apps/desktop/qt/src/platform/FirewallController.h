#ifndef FIREWALLCONTROLLER_H
#define FIREWALLCONTROLLER_H

#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP QP14 -- Windows Firewall and the Public-network warning for a hub that
// shares (behaviour.md §11, UPW2): whether an inbound rule lets this program in
// on Private and Domain networks, a button that asks Windows for one, and
// whether a connected network on an adapter with a gateway is set to Public.
// Windows only: elsewhere `supported` is false and nothing is read. QML sees it
// as the singleton `FirewallController`; the rows are on the Networking page.
//
// Reads and the change run on a thread of this controller's (COM, netsh and
// its UAC prompt block); results come back to the GUI thread.
class FirewallController : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool supported READ supported NOTIFY changed)
    // "blocked", "allowed", "none" (no rule for this program), "off" (the firewall
    // is off for those profiles), or "" before the first read
    Q_PROPERTY(QString rules READ rules NOTIFY changed)
    Q_PROPERTY(bool onPublicNetwork READ onPublicNetwork NOTIFY changed)
    Q_PROPERTY(bool checking READ checking NOTIFY changed)
    Q_PROPERTY(bool fixing READ fixing NOTIFY changed)
    // Why the last allow() did not change anything, "" otherwise
    Q_PROPERTY(QString fixError READ fixError NOTIFY changed)
    // False when the program's path holds a %: a rule cannot name it
    Q_PROPERTY(bool pathNameable READ pathNameable CONSTANT)
    // Re-read every 5 s while on (the page shows and sharing is on)
    Q_PROPERTY(bool watching READ watching WRITE setWatching NOTIFY watchingChanged)

public:
    FirewallController(const AppContext &ctx, QObject *parent);
    ~FirewallController() override;
    static FirewallController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<FirewallController>(); }

    bool supported() const;
    QString rules() const { return m_rules; }
    bool onPublicNetwork() const { return m_public; }
    bool checking() const { return m_checking; }
    bool fixing() const { return m_fixing; }
    QString fixError() const { return m_fixError; }
    bool pathNameable() const { return !m_exe.contains(QLatin1Char('%')); }
    bool watching() const { return m_watch.isActive(); }
    void setWatching(bool on);

    Q_INVOKABLE void check();
    // Asks Windows (one UAC prompt) for an Allow rule, then reads again
    Q_INVOKABLE void allow();
    // Windows' network status settings, where a network's profile is chosen
    Q_INVOKABLE void openNetworkSettings();

    // For the smoke driver on any OS: a state to show, as if read
    void simulate(const QString &rules, bool onPublic);

signals:
    void changed();
    void watchingChanged();

private:
    QString m_exe;
    QString m_rules;
    bool m_public = false;
    bool m_checking = false;
    bool m_fixing = false;
    bool m_simulated = false;
    QString m_fixError;
    QTimer m_watch;
    QThreadPool m_worker;
};

#endif // FIREWALLCONTROLLER_H
