#ifndef DEVICEINFOCONTROLLER_H
#define DEVICEINFOCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <cstdint>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP PGQT -- The open device's protocol (PLAN-PG wave PG-D): the rows
// Settings > About shows under "Connected Device" (generation, protocol
// version, page list, extension envelope, board), the raw request dialog by
// command id, and what the alink sender's page shows.
//
// - The protocol is the core's dspi_device_protocol, read once per connection
//   on the worker (it costs one platform read): a new connection generation
//   (dspi_connection_generation) reads it again, a snapshot of the same
//   connection does not. A paged device whose page list is empty (a hub's
//   in-process or principal connection carries no page discovery) gets a note
//   saying so, as the Terminal's (dspi-tui NO_PAGE_LIST_NOTE).
// - `sender` is the core's device kind AlinkSender (WP PGFIX): the main window
//   then shows SenderPage in place of the DSP screens (device info, the audio
//   link status, the update), since every DSP call answers Unsupported there.
// - `bootsel`: whether the board can be offered a firmware image (board id 0,
//   a Pico with its ROM bootloader). FirmwareController offers no image to a
//   board without it (a DSPi-Zero or an unknown board).
// - The raw dialog sends one paged GET or SET by command id through the core's
//   dspi_raw_get_id / dspi_raw_set_id, and says the core's refusals in words.
//   Anything that would go out truncated is refused before the core is asked:
//   an id, wValue or length above 16 bits, a byte above 0xFF, a payload longer
//   than a control transfer carries. A SET goes through DeviceModel::write (the
//   one write path); dspi_raw_set_id reads SYS.COMMAND_INFO per call.
// - -- WP QTSENDER -- On the sender only the action ids of senderKeeps() do
//   anything useful: its device info (Settings, Help > About), the audio link
//   (Window > System...), the firmware update, the connection (Devices...,
//   Reconnect, Open File... for an offline document) and the app's own rows
//   (Quit, Help). Every other menu row, its shortcut and its tool window is
//   greyed (MenuTable.available, ToolWindow.senderLocked), and Shell.trigger
//   refuses it (the gate installed here), whoever triggers it.
class DeviceInfoController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // A record was read for the open connection
    Q_PROPERTY(bool known READ known NOTIFY changed)
    // [{key, label, value}] in the Terminal's order: board, generation,
    // protocol, pages, extensions
    Q_PROPERTY(QVariantList rows READ rows NOTIFY changed)
    Q_PROPERTY(QStringList notes READ notes NOTIFY changed)
    // "legacy", "paged", "unknown", or "" with nothing read
    Q_PROPERTY(QString generation READ generation NOTIFY changed)
    Q_PROPERTY(bool paged READ paged NOTIFY changed)
    Q_PROPERTY(bool sender READ sender NOTIFY changed)
    Q_PROPERTY(int boardId READ boardId NOTIFY changed)
    Q_PROPERTY(QString boardName READ boardName NOTIFY changed)
    Q_PROPERTY(bool bootsel READ bootsel NOTIFY changed)
    // ---- The raw dialog ----
    Q_PROPERTY(bool rawBusy READ rawBusy NOTIFY rawChanged)
    // The last request's outcome: a hex dump or "Sent ...", or the refusal
    Q_PROPERTY(QString rawResult READ rawResult NOTIFY rawChanged)
    Q_PROPERTY(bool rawFailed READ rawFailed NOTIFY rawChanged)
    // ---- The alink sender's page ----
    // [{label, value}] of the last dspi_fetch_alink_status
    Q_PROPERTY(QVariantList alinkRows READ alinkRows NOTIFY alinkChanged)
    Q_PROPERTY(QString alinkProblem READ alinkProblem NOTIFY alinkChanged)
    Q_PROPERTY(bool alinkBusy READ alinkBusy NOTIFY alinkChanged)
public:
    DeviceInfoController(const AppContext &ctx, QObject *parent);
    static DeviceInfoController *create(QQmlEngine *, QJSEngine *);

    bool known() const { return !m_protocol.isEmpty(); }
    QVariantList rows() const { return m_rows; }
    QStringList notes() const { return m_notes; }
    QString generation() const;
    bool paged() const { return generation() == QLatin1String("paged"); }
    bool sender() const;
    int boardId() const;
    QString boardName() const;
    bool bootsel() const { return known() && boardId() == 0; }
    const QJsonObject &protocol() const { return m_protocol; }
    // dspi_device_protocol calls so far (the smoke driver: once per connection)
    int reads() const { return m_reads; }

    bool rawBusy() const { return m_rawBusy; }
    QString rawResult() const { return m_rawResult; }
    bool rawFailed() const { return m_rawFailed; }
    // One paged GET: `id` 0xPPCC, `value` the wValue ("" for 0), `length` the bytes asked
    Q_INVOKABLE void rawGet(const QString &id, const QString &value, const QString &length);
    // One paged SET: `hex` the payload ("" for none: an action)
    Q_INVOKABLE void rawSet(const QString &id, const QString &value, const QString &hex);

    QVariantList alinkRows() const { return m_alinkRows; }
    QString alinkProblem() const { return m_alinkProblem; }
    bool alinkBusy() const { return m_alinkBusy; }
    // Reads the audio link status again (on the worker)
    Q_INVOKABLE void refreshAlink();

    // -- WP QTSENDER -- Whether menu action `action` does something on the
    // alink sender (whatever device is open: callers check `sender` first)
    Q_INVOKABLE bool senderKeeps(const QString &action) const;

    // ---- Pure, for the smoke driver ----
    struct RawRequest {
        QString error;   // "" when the request may go to the core
        uint16_t id = 0;
        uint16_t value = 0;
        uint16_t length = 0;
        QByteArray data;
    };
    static RawRequest parseRaw(bool set, const QString &id, const QString &value, const QString &lengthOrHex);
    // The core's failure reply for command `id` in words
    static QString refusalWords(uint16_t id, const QJsonObject &reply);
    // The rows and notes of a dspi_device_protocol reply
    static QVariantList rowsOf(const QJsonObject &protocol);
    static QStringList notesOf(const QJsonObject &protocol);
    static QString hexDump(const QByteArray &bytes);

signals:
    void changed();
    void rawChanged();
    void alinkChanged();

private:
    void onSnapshot();
    void read();
    void clear();
    void rawDone(bool failed, const QString &text);

    AppContext m_ctx;
    QJsonObject m_protocol;
    QVariantList m_rows;
    QStringList m_notes;
    uint64_t m_readFor = 0;    // the connection the record is of
    bool m_reading = false;
    int m_reads = 0;
    bool m_rawBusy = false;
    QString m_rawResult;
    bool m_rawFailed = false;
    quint64 m_rawCount = 0;
    QVariantList m_alinkRows;
    QString m_alinkProblem;
    bool m_alinkBusy = false;
};

#endif // DEVICEINFOCONTROLLER_H
