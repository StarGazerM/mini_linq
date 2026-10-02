//! Retained nonemptiness over unit rows; evaluation consumes the complete source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exists(bool);
impl FromIterator<()> for Exists {
    fn from_iter<I: IntoIterator<Item = ()>>(source: I) -> Self {
        let mut exists = false;
        for () in source {
            exists = true;
        }
        Self(exists)
    }
}
impl Exists {
    pub fn iter(&self) -> std::slice::Iter<'static, ()> {
        if self.0 { [()].iter() } else { [].iter() }
    }
}
