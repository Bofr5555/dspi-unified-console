#ifndef FIREWALLCONTROLLER_H
#define FIREWALLCONTROLLER_H

#include <QObject>
#include <QString>
#include <QThreadPool>

class DSPiBridge;

// WP UPW2: whether Windows lets other Consoles reach this one's hub, for Settings >
// Networking while sharing (HW-TERMINAL.md:98). QML sees it as `firewall`.
//
// Two readings, both through the Windows APIs: Windows Firewall's inbound rules for
// this program (INetFwPolicy2), judged for the Private and Domain profiles, and
// whether a connected network on an adapter with a default gateway is Public
// (INetworkListManager, iphlpapi). A Cancel on
// Windows' first question leaves Block rules, which win over any Allow rule.
//
// allow() turns this program's inbound Block rules into Allow rules for Private and
// Domain networks, or adds one such rule when there is none: netsh from the system
// directory, run elevated after a UAC prompt the user confirms (Windows shows it as
// "Network Command Shell"). Nothing adds a rule without that (the installer adds none).
//
// Reads and the fix run on a worker thread; elsewhere than on Windows `supported`
// is false and nothing runs.
class FirewallController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool supported READ supported CONSTANT)
    // "" not read yet or unknown; "allowed"; "blocked" (a Block rule); "none" (no
    // rule: Windows blocks it and may ask); "off" (the firewall is off for them)
    Q_PROPERTY(QString rules READ rules NOTIFY changed)
    Q_PROPERTY(bool publicNetwork READ publicNetwork NOTIFY changed)
    Q_PROPERTY(bool checking READ checking NOTIFY changed)
    // The UAC prompt or netsh is running
    Q_PROPERTY(bool fixing READ fixing NOTIFY changed)
    // Why the last allow() did not change the rules, as a sentence; "" otherwise
    Q_PROPERTY(QString fixError READ fixError NOTIFY changed)

public:
    FirewallController(DSPiBridge *bridge, QObject *parent);
    ~FirewallController() override;

    bool supported() const;
    QString rules() const { return m_rules; }
    bool publicNetwork() const { return m_public; }
    bool checking() const { return m_checking; }
    bool fixing() const { return m_fixing; }
    QString fixError() const { return m_fixError; }

    // Read both again; a read already on its way covers this call
    Q_INVOKABLE void check();
    Q_INVOKABLE void allow();
    Q_INVOKABLE void openNetworkSettings();

signals:
    void changed();

private:
    QString m_exe;
    QString m_rules;
    bool m_public = false;
    bool m_checking = false;
    bool m_fixing = false;
    QString m_fixError;
    QThreadPool m_worker;               // one thread, joined by the destructor
};

#endif // FIREWALLCONTROLLER_H
