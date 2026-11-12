#ifndef EXTCONTROLLER_H
#define EXTCONTROLLER_H

#include <QByteArray>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "CoreApi.h"
#include "Registrar.h"

class QJSEngine;

// -- WP QP7 -- The extension channel (REQ_EXTENSION, the core's ext.rs) as the
// crossfeed window uses it: what the device lists, and the crossfeed pair map
// (EXT_CROSSFEED_PAIR_MAP, 0x0001: which two outputs each pair processes).
// The Mac's ExtensionCommands and CrossfeedPairMapView.
//
// The status is the core's probe from the connect (`dspi_ext_status`, no
// traffic). Without a usable pair map (stock firmware stalls the probe, or a
// hub refuses this client's role) hasPairMap is false and the window shows
// none of it. The map is taken from each snapshot's bulk image, where it
// rides (so a preset load or another console's change shows with no extra
// read); a write sends the whole map (pairs stay disjoint), the core reads it
// back, and the window shows what the device kept.
//
// Map bytes: byte p is pair p; 0 is the default pairing (outputs 2p, 2p+1),
// otherwise (right << 4) | left, outputs numbered from 0.
class ExtController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The probe has answered for this connection
    Q_PROPERTY(bool known READ known NOTIFY changed)
    Q_PROPERTY(bool hasPairMap READ hasPairMap NOTIFY changed)
    Q_PROPERTY(bool hasCoreVreg READ hasCoreVreg NOTIFY changed)
    // The listed sub-command ids
    Q_PROPERTY(QVariantList supported READ supported NOTIFY changed)
    // [{left, right}] per pair of this platform, outputs from 0
    Q_PROPERTY(QVariantList pairs READ pairs NOTIFY changed)
    Q_PROPERTY(int pairCount READ pairCount NOTIFY changed)
    // "default", "k340" or "custom": which of the Outputs menu's rows the map is
    Q_PROPERTY(QString choice READ choice NOTIFY changed)
    // The pair mask that goes with the K340's map (both pairs)
    Q_PROPERTY(int k340PairMask READ k340PairMask CONSTANT)
    // A write or read of the map is under way
    Q_PROPERTY(bool busy READ busy NOTIFY changed)
public:
    ExtController(const AppContext &ctx, QObject *parent);
    static ExtController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<ExtController>(); }

    bool known() const { return m_known; }
    bool hasPairMap() const { return m_hasPairMap; }
    bool hasCoreVreg() const { return m_hasCoreVreg; }
    QVariantList supported() const { return m_supported; }
    QVariantList pairs() const;
    int pairCount() const;
    QString choice() const;
    static int k340PairMask();
    bool busy() const { return m_busy > 0; }

    // Whether the editor's pairs ([{left, right}], one per pair) are a map the
    // firmware takes: two different outputs each, no output in two pairs.
    Q_INVOKABLE bool pairsValid(const QVariantList &pairs) const;
    // "default" or "k340"; false, with nothing sent, when there is no pair map
    Q_INVOKABLE bool applyChoice(const QString &choice);
    // The editor's Apply; false, with nothing sent, for pairs pairsValid refuses
    Q_INVOKABLE bool applyPairs(const QVariantList &pairs);
    // Reads the status and the map again
    Q_INVOKABLE void refresh();

    // The map's bytes as shown (normalised)
    QByteArray map() const { return m_map; }
    // For the tests: the bytes of `pairs`, or empty when they are not valid
    QByteArray encode(const QVariantList &pairs) const;
    static QByteArray normalized(const QByteArray &map);

    // Smoke build: answer as if the probe had given `json` (a stall's
    // {"ok":true,"supported":[],"hasPairMap":false,...}); "" ends it.
    void setStatusOverride(const QByteArray &json);

signals:
    void changed();
    // A write the device refused or did not keep
    void writeFailed();

private:
    bool write(const QByteArray &map);
    void readStatus(const QByteArray &json);
    // The map from the last snapshot's bulk image; true when it changed
    bool takeMap();

    AppContext m_ctx;
    bool m_known = false;
    bool m_hasPairMap = false;
    bool m_hasCoreVreg = false;
    QVariantList m_supported;
    QByteArray m_map = QByteArray(4, '\0');
    QByteArray m_override;
    bool m_open = false;
    uint64_t m_generation = 0;
    int m_busy = 0;
};

#endif // EXTCONTROLLER_H
