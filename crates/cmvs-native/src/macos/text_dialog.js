function run(argv) {
    var app = Application.currentApplication();
    app.includeStandardAdditions = true;
    app.activate();
    try {
        var answer = app.displayDialog(argv[1], {
            withTitle: argv[0], defaultAnswer: argv[2],
            buttons: ['Cancel', 'OK'], defaultButton: 'OK', cancelButton: 'Cancel'
        });
        return 'TEXT:' + answer.textReturned;
    } catch (error) {
        if (error.errorNumber === -128) return 'CANCEL';
        throw error;
    }
}
