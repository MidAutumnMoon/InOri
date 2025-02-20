# rpgdemake

A simple CLI tool for batch decrypting RPG Maker MV/MZ assets,
aiming for "blazing fast".

Based on

- https://gitgud.io/softashell/rpgmv-decrypter (<- too slow)
- https://gitlab.com/Petschko/RPG-Maker-MV-Decrypter (<- JavaScript & Web only)

## Notes on Aspects of RPG Maker

### RPG Maker Versions

Chronologically, the RPG Maker versions can be
ordered as `2000`, `XP`, `VX/VX Ace`, `MV`, `MZ`,
where starting from MV it switched from
*custom engine + Ruby(RGSS)* used for decades to
the modern *nw.js + JavaScript* stack.

This change improves the portability drastically,
as the old custom renderer which works perfectly
on Windows Not-Newer-Than-XP is replaced with Pixi + Chromium,
and the Ruby-alike-but-not-Ruby RGSS script language, which
bites really really hard when one runs into encoding problems,
is replaced with standard JavaScript.

Since one of the main goals of MV and MZ is to enable
creators to port games cross-platform with little effort,
the game now is essentially a folder with index.html,
which lets it to be played on any recent enough browser
by simply setting up a HTTP server ...under the assumption
that neither the creator nor any plugin uses non portable API,
although RPG Maker does provide some polyfill alternatives.

As a result of all that, RPG Maker MV/MZ doesn't bundle data
and resources together but lay them plainly on fs,
which works bloody poorly on Microsoft Windows when larger games
have tens of thousand small files to be extracted,
and also makes this tool easier to implement :)

There are existing tools to extract resources from
earlier RPG Maker games so this tool will focus on
dealing with MV and MZ, which'll be referred as "RPGM" later on.


## Layout of RPGM Game

Published MV and MV game share almost the same structure.

Typical MV game looks like:

``text
<root>
    - Game.exe and bunch of nw.js stuff
    - www/
        - index.html
        - package.json
        - data/ all of the JSONs
        - js/ all the js
        - img/ audio/ etc. resources
``

Where the entierty of the game is stored in the `www` folder.

On the other hand, MZ has everything in `www` but puts them
alongside with Game.exe, like how amoeba put organs in body fluid
with no separation, which is a lot messier.

```text
<root>
    - Game.exe etc.
    - index.html
    - data/ js/ img/ etc. etc.
```


## Encryption of Resources

The stock RPGM uses a naive yet effective encryption method,
as effective as putting door key under welcome carpet,
which this tool's creator is so glad of.

Currently only image and audio will be encrypted,
specificly only PNG and OGG/M4A cuz of course it's hardcoded.
MV and MV uses the exact same algorithm but with
different file extension.

In MV the encrypted PNG file has extension *rpgmvp*
and OGG file has "rpgmvo", whereas MZ uses "png_" and "ogg_"
in respect. The encrypted file as whole will be
referred as "encfile" later on.

The encryption algo at its core is simply XOR (ba dum tss).

The first 16 bytes of encfile is its own header, composed
from SIGNATURE, VER and REMANING according to rpg_core.js.
The next 16 bytes is the original file header XOR-ed
with a random encryption key, making it trivially reversable.
The number "16" is called "headerlength" is rpg_core.js.
The remaning content is leaved untouched.

But here is a big oversight that a decrypter could
just discard the first 32 bytes which are the headers and
smash a corresponding PNG or OGG header onto the remanings,
and the file is essentially "decrypted". This renders
the encryption key totally useless, and it's how
"Petschkos RPG-Maker MV & MZ-File Decrypter"
can do a keyless restoration.

Besides the stock RPGM "encryption", there are several
third-party tools that are not hard to defeat, although
it's beyond the consideration of this simple tool.


### Encryption Key

Although the encryption key is seemingly useless,
there are few tricks to it.

Generally speaking, the 16 bytes encryption key is stored
in "data/System.json" in a field named "encryptionKey"
in its hex representation. The tricky part is that System.json
may or may not be straightforward plain text.

Among all errors encountered trying to read System.json,
the most common one is that System.json contains
byte order mark, which nw.js is OK with somehow.

The next common one is that the content of System.json
is being lz-string-ed possibly as some sort of encryption
which doesn't do the job at all. Some game went a step futher
(e.g. OMORI) to change the encryption method, but the ideas
are all the same.

And after those there is Enigma Virtual Box.


## Enigma Virtual Box

Again, Enigma Virtual Box (abbreviated as EnigmaVB)
is technically not an encryption, rather a way to bundle
resources with executable with optional compression,
akin to AppImage or UPX.

In this case, if System.json is embedded in Game.exe
and not compressed, getting the key is still trivial
by simply regex against Game.exe.


## Beyond RPG Maker

At the time of writting (2024 summer), it's been almost
5 years since MZ's release, the latest "RPG Maker"
is just a plugin for Unity, which sells and receives
really poorly among the community. Even worse, the next
major release "RPG Maker WITH" (2024 autumn)
will be console only for yet another whatever reason.

Seeing lot of creators switching to Unity, and
with the raising of Godot and Bakin etc. excellent game engines,
future of RPG Maker isn't clear at all.

