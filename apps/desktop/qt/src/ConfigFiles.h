#ifndef CONFIGFILES_H
#define CONFIGFILES_H

#include <QObject>
#include <QUrl>
#include <QVariantMap>

class DSPiBridge;

// Device configuration files (.dspipreset, shared with the macOS and Windows
// Consoles) and filter text files (REW / AutoEQ compatible): reading and
// writing the files, and handing their text to the core. Results are maps
// for the QML dialogs: {ok, error, ...}.
class ConfigFiles : public QObject
{
    Q_OBJECT
public:
    explicit ConfigFiles(DSPiBridge *bridge, QObject *parent = nullptr);

    // {ok, error, message}
    Q_INVOKABLE QVariantMap exportConfiguration(const QUrl &file);
    // {ok, error, name, platform, firmware, saved, crossPlatform}
    Q_INVOKABLE QVariantMap inspectConfiguration(const QUrl &file);
    // {ok, error, lines, clean}
    Q_INVOKABLE QVariantMap importConfiguration(const QUrl &file, bool volumes, bool hardware);

    Q_INVOKABLE QVariantMap exportFilters(const QUrl &file);
    // {ok, error, kind ("native" / "rew"), channels: [{wire, label, bands, checked}], filters, preamp}
    Q_INVOKABLE QVariantMap inspectFilters(const QUrl &file);
    // The file last inspected, to the chosen channels (wire indices): {ok, error, lines}
    Q_INVOKABLE QVariantMap importFilters(const QVariantList &wires);

    // The reply buffer of the two imports. The smoke driver shrinks it to get a
    // reply too long, which real files don't reach.
    void setApplyReplyCapacity(int bytes) { m_applyCapacity = bytes; }

    // Whether an export's text is the core's refusal, its JSON failure reply, and
    // not a file to write: a configuration document has no "ok" key and a filter
    // file is not JSON. No text at all counts as one. `result` is then the reply for
    // QML. Public for the smoke driver: the bridge checks for a device first, so
    // only firmware whose image can't be mapped reaches this, and the demo has none.
    static bool exportRefused(const QByteArray &text, QVariantMap &result);

private:
    bool readFile(const QUrl &file, QByteArray &out, QString &error);
    bool writeFile(const QUrl &file, const QByteArray &data, QString &error);

    DSPiBridge *m_bridge;
    QByteArray m_filterText;     // the file the channel picker is showing
    QByteArray m_configText;     // the configuration being imported
    int m_applyCapacity = 64 * 1024;
};

#endif // CONFIGFILES_H
