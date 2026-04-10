pub mod tags {

    pub static INVALID_CHARS: &[&str] = &["<", ">", ":", "\"", "\\", "|", "?", "*"]; // '/'

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Tags {
        pub artist: String,
        pub album_artist: String,
        pub album: String,
        pub year: String,
        pub title: String,
        pub track_number: String,
        pub has_art: bool,
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
        pub fn remove_invalid_symbols(&mut self) {
            INVALID_CHARS.iter().for_each(|sym| {
                self.artist = self.artist.replace(sym, "");
                self.album_artist = self.album_artist.replace(sym, "");
                self.title = self.title.replace(sym, "");
                self.year = self.year.replace(sym, "");
            });
        }
    }
}
