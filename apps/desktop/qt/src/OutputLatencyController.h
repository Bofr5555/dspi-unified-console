#ifndef OUTPUTLATENCYCONTROLLER_H
#define OUTPUTLATENCYCONTROLLER_H

#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QVariantList>
#include <atomic>
#include <functional>
#include <vector>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// The output latency target on Global Parameters. The options come from the
// device, read again when the source changes or the page opens, as what each
// gives depends on both.
//
// A choice is a draft until save(), as on the Mac. The firmware applies a request
// from its main loop (a flash write and an output restart) and until then reports
// the old id with `pending` set. So apply() watches `pending` on the worker, and
// the draft stays on the row until the device reports the new id: the row never
// shows the old value in between. The wait is bounded and the request goes once.
// One read follows a wait that ran out, in case the device applied it just after.
class OutputLatencyController : public QObject
{
    Q_OBJECT
    // False on firmware without the setting, and with no device
    Q_PROPERTY(bool supported READ supported NOTIFY storedChanged)
    Q_PROPERTY(int requestedId READ requestedId NOTIFY storedChanged)
    // {text, value} for the row's menu, a "-" entry added for an id not in the table
    Q_PROPERTY(QVariantList choices READ choices NOTIFY shownChanged)
    // What the row shows: the draft, or what is stored
    Q_PROPERTY(int shownId READ shownId NOTIFY shownChanged)
    Q_PROPERTY(QString caption READ caption NOTIFY shownChanged)
    Q_PROPERTY(bool dirty READ dirty NOTIFY shownChanged)
    Q_PROPERTY(bool applying READ applying NOTIFY applyingChanged)

public:
    explicit OutputLatencyController(DSPiBridge *bridge, QObject *parent = nullptr);
    ~OutputLatencyController() override;

    bool supported() const { return m_supported; }
    int requestedId() const { return m_requested; }
    QVariantList choices() const;
    int shownId() const { return m_held ? m_draft : m_requested; }
    QString caption() const;
    bool dirty() const { return m_supported && m_held && m_draft != m_requested; }
    bool applying() const { return m_applying; }

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void edit(int id);
    Q_INVOKABLE void save();
    Q_INVOKABLE void revert();

    // For the smoke driver: runs on the worker before the write. False refuses
    // it, as the device would.
    void setWriteHook(std::function<bool()> hook) { m_writeHook = std::move(hook); }

signals:
    void storedChanged();
    void shownChanged();
    void applyingChanged();
    // The device did not take a save; the draft stays
    void saveFailed(const QString &reason);
    // A read found the device now holds the draft, so saveFailed no longer stands
    void draftSettled();

private:
    struct Option {
        int id = 0;
        int latencyUs = 0;
        int effectiveUs = 0;
        bool available = false;
    };
    enum class Read { Ok, Unsupported, Failed };
    struct Status {
        Read result = Read::Failed;
        int requested = 0;
        bool pending = false;
    };
    struct Reading {
        Status status;
        bool haveOptions = false;
        std::vector<Option> options;
    };
    enum class Outcome { Applied, NotSent, NotYetApplied, Kept };

    static Status readStatus(FfiCore *core);
    static Reading read(FfiCore *core);

    void sync();
    void clearStored();
    void install(const Reading &reading);
    void apply(int id);
    void applied(Outcome outcome, const Reading &reading);
    const Option *optionFor(int id) const;

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    QString m_serial;
    int m_inputSource = -1;
    bool m_supported = false;
    int m_requested = 0;
    std::vector<Option> m_options;
    bool m_held = false;
    int m_draft = 0;
    bool m_applying = false;
    std::atomic<unsigned> m_generation { 0 };   // a read or a wait stops at a change of device
    std::function<bool()> m_writeHook;
    QThreadPool m_worker;                       // one thread, joined by the destructor
};

#endif
