use bevy_ecs::system::{Local, SystemParam};

#[derive(SystemParam)]
pub struct FirstRun<'s>(Local<'s, bool>);

impl FirstRun<'_> {
    pub fn is_first_run(&mut self) -> bool {
        if !*self.0 {
            *self.0 = true;
            true
        } else {
            false
        }
    }
}
