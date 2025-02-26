{
    users.users."name".links = {
        xdg.config."termina/app.ini" = ''
            some some text
        '';
        home.file.".local/bin".source = "{{ HOME }}";
        home.file.".config/environment".text = '''';
    };

    output = {
        meta.version = 1;
        links = [
            {
                source = "nix store";
                target = "/home/teapot/.config/environment.d";
            }
        ];
    };
}
