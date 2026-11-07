#ifndef NAMESCONTROLLER_H
#define NAMESCONTROLLER_H

#include <QObject>
#include <QStringList>
#include <QThreadPool>
#include <atomic>
#include <functional>
#include <vector>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// The names the device holds: one per input source, and its own. What is stored
// comes from the core's cache, so showing it costs no traffic: connect fills the
// cache and notifications keep it current, from any client. Each batch of
// notifications compares it here, so another client's rename shows without this
// needing to know why the batch came.
//
// Edits are drafts until save(), as on the Mac. All writes go through
// saveDrafts(), in order, off the UI thread.
class NamesController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool sourceNamesSupported READ sourceNamesSupported NOTIFY storedChanged)
    Q_PROPERTY(bool deviceNameSupported READ deviceNameSupported NOTIFY storedChanged)
    // Indexed by input source
    Q_PROPERTY(QStringList sourceNames READ sourceNames NOTIFY storedChanged)
    Q_PROPERTY(QString deviceName READ deviceName NOTIFY storedChanged)
    // What each field shows: the draft, or what is stored
    Q_PROPERTY(QStringList sourceTexts READ sourceTexts NOTIFY textsChanged)
    Q_PROPERTY(QString deviceText READ deviceText NOTIFY textsChanged)
    Q_PROPERTY(bool dirty READ dirty NOTIFY textsChanged)
    Q_PROPERTY(bool saving READ saving NOTIFY savingChanged)

public:
    static constexpr int kSourceCount = 7;   // 0..=INPUT_SOURCE_MAX

    explicit NamesController(DSPiBridge *bridge, QObject *parent = nullptr);
    ~NamesController() override;

    bool sourceNamesSupported() const { return m_sourcesSupported; }
    bool deviceNameSupported() const { return m_deviceSupported; }
    QStringList sourceNames() const { return m_sources; }
    QString deviceName() const { return m_device; }
    QStringList sourceTexts() const;
    QString deviceText() const;
    bool dirty() const;
    bool saving() const { return m_saving; }

    // What the device can store and its front panel draw: printable ASCII, at
    // most 31 characters
    Q_INVOKABLE QString clean(const QString &text) const;
    Q_INVOKABLE bool sourcePending(int source) const;
    Q_INVOKABLE bool devicePending() const;
    Q_INVOKABLE void editSource(int source, const QString &text);
    Q_INVOKABLE void editDevice(const QString &text);
    Q_INVOKABLE void save();
    Q_INVOKABLE void revert();

    // For the smoke driver: runs on the worker before each write. False refuses
    // that write, as the device would.
    void setWriteHook(std::function<bool()> hook) { m_writeHook = std::move(hook); }

signals:
    void storedChanged();
    void textsChanged();
    void savingChanged();
    // The device refused a save; the drafts stay
    void saveFailed(const QString &reason);

private:
    // What a save sends. A `source` of -1 means the device name.
    struct Write {
        int source;
        QByteArray name;
    };
    struct Draft {
        bool held = false;
        QString text;
    };

    void sync();
    void refresh();
    void clearDrafts();
    void saveDrafts(const std::vector<Write> &writes);
    void saveDone(const std::vector<Write> &writes, bool ok);

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    QString m_serial;
    bool m_sourcesSupported = false;
    bool m_deviceSupported = false;
    QStringList m_sources;
    QString m_device;
    Draft m_sourceDrafts[kSourceCount];
    Draft m_deviceDraft;
    bool m_saving = false;
    std::atomic<unsigned> m_generation { 0 };   // a save stops at a change of device
    std::function<bool()> m_writeHook;
    QThreadPool m_worker;                       // one thread, joined by the destructor
};

#endif
