pub mod tags {

    pub static INVALID_CHARS: &[&str] = &["<", ">", ":", "\"", "\\", "|", "?", "*"]; // '/'

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ArtistMetadata {
        pub artist: String,
    }
    impl ArtistMetadata {
        pub fn get_from_tags(tags: &Tags) -> ArtistMetadata {
            // the artist directory is named after 'artist', album artist is only a fallback
            let artist = if tags.artist.is_empty() {
                tags.album_artist.clone()
            } else {
                tags.artist.clone()
            };

            ArtistMetadata { artist }
        }

        pub fn get_from_album(album: &AlbumMetadata) -> ArtistMetadata {
            album.artist.clone()
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AlbumMetadata {
        pub artist: ArtistMetadata,

        pub album_name: String,
        pub year: String,
        pub has_art: bool,
    }
    impl AlbumMetadata {
        pub fn get_from_tags(tags: &Tags) -> AlbumMetadata {
            AlbumMetadata {
                artist: ArtistMetadata::get_from_tags(tags),
                album_name: tags.album.clone(),
                year: tags.year.clone(),
                has_art: tags.has_art,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Tags {
        pub artist: String,
        pub album_artist: String,

        pub album: String,
        pub year: String,
        pub has_art: bool,

        pub title: String,
        pub track_number: String,

        // Part of a set: which disc this file sits on and how many there are.
        // Both are empty when the file says nothing, and only a set of more
        // than one disc ever reaches the file name.
        pub disc_number: String,
        pub total_discs: String,
    }
    impl Tags {
        pub fn remove_slashes(&mut self) {
            self.artist = self.artist.replace("/", "-");
            self.album = self.album.replace("/", "-");
            self.album_artist = self.album_artist.replace("/", "-");
            self.title = self.title.replace("/", "-");
            self.year = self.year.replace("/", "-");
        }
        pub fn remove_null_bytes(&mut self) {
            self.artist = self.artist.replace("\0", "");
            self.album = self.album.replace("\0", "");
            self.album_artist = self.album_artist.replace("\0", "");
            self.title = self.title.replace("\0", "");
            self.year = self.year.replace("\0", "");
        }
        // The album belongs here as much as every other name: it is a whole
        // component of a path, and leaving it out kept a character in the tag
        // that 'FixCharacters' had just taken out of the directory, so the two
        // could never agree again.
        pub fn remove_invalid_symbols(&mut self) {
            INVALID_CHARS.iter().for_each(|sym| {
                self.artist = self.artist.replace(sym, "");
                self.album_artist = self.album_artist.replace(sym, "");
                self.album = self.album.replace(sym, "");
                self.title = self.title.replace(sym, "");
                self.year = self.year.replace(sym, "");
            });
        }
    }
}
