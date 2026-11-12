#ifndef DASHBOARDCONTROLLER_H
#define DASHBOARDCONTROLLER_H

#include <QElapsedTimer>
#include <QJsonArray>
#include <QObject>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// The Overview in the right pane (the Mac's DashboardOverview): which cards it
// shows, how many per row, the clip marks and whether the meters are current.
//
// Cards: the stereo input card (inputs 1 and 2), then the core's output cards
// (dspi_output_cards: paired by the crossfeed pair map when the device has
// one, else by slot; a pair with one side off shows the other alone; the PDM
// output alone). Each card is a list of columns, each a wire channel with its
// heading, colour key and delay readout.
//
// Meters: the base's 60 ms meter poll (Poller.peaks, clipFlags) feeds every
// meter; this controller adds no read of its own. Clip flags are latched here
// as the Mac does: a channel's mark stays until 10 s pass with no new clip,
// then the marks and the device's flags are cleared.
//
// Stale: while a device is open, `stale` is set when no meter reading has
// arrived for a while (reads failing or not answering). The meters keep the
// last values read and are never greyed; the Overview shows a note.
class DashboardController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QVariantList cards READ cards NOTIFY cardsChanged)
    Q_PROPERTY(int cardsPerRow READ cardsPerRow WRITE setCardsPerRow NOTIFY cardsPerRowChanged)
    Q_PROPERTY(uint clipLatched READ clipLatched NOTIFY clipLatchedChanged)
    Q_PROPERTY(bool stale READ stale NOTIFY staleChanged)
    Q_PROPERTY(int spacing READ spacing CONSTANT)
public:
    // The Mac's layout constants: gap between cards, and the narrowest card
    // for which Auto still puts another one in the row
    static constexpr int kSpacing = 18;
    static constexpr int kAutoMinCardWidth = 440;
    // The largest fixed choice in the layout popup
    static constexpr int kMaxCardsPerRow = 3;
    // A latched clip mark clears after this long without a new clip
    static constexpr int kClipHoldMs = 10000;
    // No meter reading for this long (at least, see staleAfterMs) is stale
    static constexpr int kStaleMs = 1000;

    DashboardController(const AppContext &ctx, QObject *parent = nullptr);
    static DashboardController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<DashboardController>(); }

    QVariantList cards() const { return m_cards; }
    int cardsPerRow() const { return m_cardsPerRow; }
    // 0 is Auto; 1..3 fixed. Saved as dashboard/cardsPerRow.
    void setCardsPerRow(int n);
    uint clipLatched() const { return m_clipLatched; }
    bool stale() const { return m_stale; }
    int spacing() const { return kSpacing; }

    // Cards per row for a grid `width` wide holding `count` cards, with
    // `chosen` cards per row (0 Auto): as many as fit for Auto, otherwise the
    // choice; never more than the cards.
    Q_INVOKABLE static int columnsFor(int chosen, qreal width, int count);
    // Whether wire channel `channel`'s clip mark shows
    Q_INVOKABLE bool clipped(int channel) const;

    // A band row's texts (the Mac's DashboardRow): the type's short code,
    // whether the type has a gain and a Q worth showing, and the numbers
    Q_INVOKABLE static QString typeCode(int type);
    Q_INVOKABLE static bool usesGain(int type);
    Q_INVOKABLE static bool showsQ(int type);
    Q_INVOKABLE static QString freqText(double hz);
    Q_INVOKABLE static QString gainText(double db);
    Q_INVOKABLE static QString qText(double q);
    // A delay in ms with up to four decimals, trailing zeros dropped ("1.5", "0")
    Q_INVOKABLE static QString delayText(double ms);

    // The stale threshold for the current transport, ms
    int staleAfterMs() const;

signals:
    void cardsChanged();
    void cardsPerRowChanged();
    void clipLatchedChanged();
    void staleChanged();

private:
    void fetchCards();
    void rebuild();
    void onMeters();
    void onRunning();
    void setStale(bool stale);
    QVariantMap column(int wire, const QString &heading, bool output, int number, bool showDelay) const;

    AppContext m_ctx;
    QVariantList m_cards;
    QJsonArray m_outputCards;
    bool m_cardsInFlight = false;
    bool m_cardsAgain = false;
    int m_cardsPerRow = 0;
    uint m_clipLatched = 0;
    QElapsedTimer m_lastClip;
    QTimer m_staleTimer;
    bool m_stale = false;
};

#endif // DASHBOARDCONTROLLER_H
