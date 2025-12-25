pub mod tags {

    use std::collections::HashSet;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Tags {
        pub artist: String,
        pub album_artist: String,
        pub album: String,
        pub title: String,
        pub track_number: String,
    }
    impl Tags {
        pub fn remove_slashes(&mut self) {
            self.artist = self.artist.replace("/", "-");
            self.album = self.album.replace("/", "-");
            self.album_artist = self.album_artist.replace("/", "-");
            self.title = self.title.replace("/", "-");
        }
        pub fn remove_null_bytes(&mut self) {
            self.artist = self.artist.replace("\0", "");
            self.album = self.album.replace("\0", "");
            self.album_artist = self.album_artist.replace("\0", "");
            self.title = self.title.replace("\0", "");
        }
        pub fn remove_invalid_symbols(&mut self) {
            let invalid_symbols = HashSet::from(["<", ">", ":", "\"", "/", "\\", "|", "?", "*"]);
            invalid_symbols.iter().for_each(|sym| {
                self.artist = self.artist.replace(sym, "");
                self.album_artist = self.album_artist.replace(sym, "");
                self.title = self.title.replace(sym, "");
            });
        }

        pub fn fix_track_number(&mut self) {
            self.track_number = self
                .track_number
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
        }
    }
}
