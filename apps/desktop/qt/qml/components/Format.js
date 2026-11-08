.pragma library
// -- WP QD40 -- A translated template's %1, %2 ... filled in one pass. Chained
// .arg() reads what an earlier argument put in, so a channel named "Left %2"
// came out with the next argument inside it; text put in here stays as it is.
// (QML's String.arg takes one value, so there is no .arg(a, b) to use instead.)
function fmt(template, args) {
    return String(template).replace(/%(\d+)/g, function (whole, n) {
        var i = Number(n) - 1
        return i >= 0 && i < args.length ? String(args[i]) : whole
    })
}
