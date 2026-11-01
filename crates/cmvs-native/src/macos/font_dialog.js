ObjC.import('AppKit');

function run(argv) {
    var manager = $.NSFontManager.sharedFontManager;
    var families = ObjC.deepUnwrap(manager.availableFontFamilies).filter(function (name) {
        if (!name || name[0] === '@' || /[\x00-\x1f\x7f]/.test(name)) return false;
        var font = manager.fontWithFamilyTraitsWeightSize($(name), 0, 5, 12);
        if (!font || font.isNil()) return false;
        var charset = font.coveredCharacterSet;
        for (var c = 32; c <= 126; c++) {
            if (!charset.longCharacterIsMember(c)) return false;
        }
        // A practical Japanese coverage probe, not a guarantee for every game glyph.
        return [0x3042, 0x30a2, 0x65e5, 0x672c].every(function (c) {
            return charset.longCharacterIsMember(c);
        });
    }).sort();
    if (!families.length || families.length > 8192) throw Error('No usable Japanese font families');
    var app = Application.currentApplication();
    app.includeStandardAdditions = true;
    app.activate();
    var options = {withTitle: 'CMVS font', withPrompt: 'Choose a Japanese font family',
                   multipleSelectionsAllowed: false, emptySelectionAllowed: false};
    if (families.indexOf(argv[0]) >= 0) options.defaultItems = [argv[0]];
    var selected = app.chooseFromList(families, options);
    if (selected === false) return 'CANCEL';
    if (selected.length !== 1 || families.indexOf(selected[0]) < 0) throw Error('Invalid selection');
    return 'FONT:' + selected[0];
}
