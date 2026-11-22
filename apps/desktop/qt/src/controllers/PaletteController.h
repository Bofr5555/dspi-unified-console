#ifndef PALETTECONTROLLER_H
#define PALETTECONTROLLER_H

#include <QObject>
#include <QString>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP LAY6 -- The command palette's half that is no QML (qml/modern/
// CommandPalette.qml makes the entries and runs them): its shortcut, opening it
// from any window, and finding entries by what was typed.
//
// Finding: the query's words, each of which must be in the entry, in any order;
// case, accents and punctuation between words do not count. A word is found, best
// first, as the start of the title, the start of one of its words, inside it, as
// the first letters of its words ("sg": Signal Generator), or with letters left
// out ("cfeed": Crossfeed). An alias is another title; a keyword (a settings
// row's label, a menu's name) counts less, and only as it is spelt.
class PaletteController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Modern's, in every window; no MenuTable row and no panel may have it
    Q_PROPERTY(QString shortcut READ shortcut CONSTANT)
public:
    PaletteController(const AppContext &ctx, QObject *parent);
    static PaletteController *create(QQmlEngine *, QJSEngine *);

    static QString shortcut() { return QStringLiteral("Ctrl+K"); }

    // Asks the Modern shell for the palette. From another window the main one
    // comes to the front; while a dialog asks something there, that is all.
    Q_INVOKABLE void open();

    // The entries `query` finds, best first; all, in their order, for "".
    // An entry: { title, aliases, keywords, enabled, ... }; one that comes back
    // has `matched` besides: the alias or keyword it was found by, "" for its
    // title. A greyed one comes after its equals.
    Q_INVOKABLE QVariantList search(const QVariantList &entries, const QString &query) const;

    // How well `query` finds an entry: 0 not at all. `matched` as above
    static double score(const QVariantMap &entry, const QString &query, QString *matched = nullptr);

signals:
    void opening();
};

#endif // PALETTECONTROLLER_H
