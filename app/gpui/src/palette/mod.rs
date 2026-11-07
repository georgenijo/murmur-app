pub mod model;
pub mod host;
#[cfg(any(test,feature="gallery"))] pub mod fixtures;
#[cfg(test)] mod tests;
use crate::{section::{self,Controller},TextField,field::{InputEvent,TextFieldState},tokens::Palette};
use gpui::{prelude::*,*};
use std::rc::Rc;
use model::*;
use host::Action;
pub struct CommandPalette {
 pub controller:Controller<Snapshot,Action>,pub search:Entity<TextFieldState>,pub selected:usize,pub open:bool,
 rows:Vec<usize>,scroll:ScrollHandle,return_focus:Option<FocusHandle>,_input:Subscription,
}
impl CommandPalette {
 pub fn new(host:Rc<host::Host>,w:&mut Window,cx:&mut Context<Self>)->Self{
 let search=cx.new(|cx|TextFieldState::new(w,cx).placeholder("Type a command…"));
 let input=cx.subscribe(&search,|s,_,e,cx|{if matches!(e,InputEvent::Change){s.selected=0;s.scroll.set_offset(point(px(0.),px(0.)));cx.notify();}});
 Self{controller:Controller::new(host),search,selected:0,open:false,rows:vec![],scroll:ScrollHandle::new(),return_focus:None,_input:input}
 }
 pub fn attach(&mut self,cx:&mut Context<Self>){self.controller.attach(Action::Read);self.controller.drain();cx.notify();}
 pub fn follow(&self,cx:&mut Context<Self>){section::follow(cx,self.controller.model.attachment,|s|&mut s.controller)}
 pub fn show(&mut self,w:&mut Window,cx:&mut Context<Self>){self.return_focus=w.focused(cx);self.search.update(cx,|s,cx|s.set_value("",w,cx));self.selected=0;self.open=true;w.focus(&self.search.read(cx).focus_handle(cx),cx);cx.notify();}
 pub fn close(&mut self,w:&mut Window,cx:&mut Context<Self>){self.open=false;self.controller.invalidate();if let Some(f)=self.return_focus.take(){w.focus(&f,cx)}cx.notify();}
 pub fn hide(&mut self,w:&mut Window,cx:&mut Context<Self>){self.close(w,cx);self.controller.detach();self.search.update(cx,|s,cx|s.set_value("",w,cx));}
 fn run(&mut self,w:&mut Window,cx:&mut Context<Self>){
 if let Some(id)=self.rows.get(self.selected).and_then(|i|self.controller.model.snapshot.as_ref().and_then(|s|s.commands.get(*i))).map(|c|c.id.clone()){
 self.controller.request(Action::Run(id));self.open=false;if let Some(f)=self.return_focus.take(){w.focus(&f,cx)}cx.notify();}
 }
 fn key(&mut self,e:&KeyDownEvent,w:&mut Window,cx:&mut Context<Self>){
 let k=&e.keystroke;if e.prefer_character_input{return}let control=k.modifiers.control&&!k.modifiers.platform&&!k.modifiers.alt&&!k.modifiers.shift;
 match k.key.as_str(){"escape"=>self.close(w,cx),"tab"=>(),"up"|"down" if k.modifiers.number_of_modifiers()==0=>{let n=self.rows.len();if n>0{self.selected=(self.selected+n+if k.key=="up"{n-1}else{1})%n;self.scroll.scroll_to_item(self.selected);}},"p"|"n" if control=>{let n=self.rows.len();if n>0{self.selected=(self.selected+n+if k.key=="p"{n-1}else{1})%n;self.scroll.scroll_to_item(self.selected);}},"enter" if k.modifiers.number_of_modifiers()==0=>self.run(w,cx),_=>return}
 w.prevent_default();cx.stop_propagation();cx.notify();
 }
}
impl Render for CommandPalette{
 fn render(&mut self,_:&mut Window,cx:&mut Context<Self>)->impl IntoElement{
 self.controller.drain();let p=Palette::get(cx);let query=self.search.read(cx).value().to_string();
 self.rows=self.controller.model.snapshot.as_ref().map_or(vec![],|s|filter(&s.commands,&query));self.selected=self.selected.min(self.rows.len().saturating_sub(1));
 let mut list=div().id("palette-list").role(Role::ListBox).aria_label("Commands").overflow_y_scroll().track_scroll(&self.scroll).max_h(px(420.)).flex().flex_col();
 if let Some(snapshot)=&self.controller.model.snapshot{for (row,index) in self.rows.iter().enumerate(){let c=&snapshot.commands[*index];let e=cx.entity();let m=e.clone();
 list=list.child(div().id(row).role(Role::ListBoxOption).aria_label(c.title.clone()).aria_selected(row==self.selected).when(row==self.selected,|d|d.aria_active_descendant().bg(p.raised)).p(px(10.)).flex().gap(px(12.)).child(div().flex_1().child(c.title.clone())).child(div().text_color(p.muted).child(c.hint.clone().unwrap_or_else(||c.section.clone()))).on_mouse_move(move|_,_,cx|m.update(cx,|s,cx|{if s.selected!=row{s.selected=row;cx.notify()}})).on_click(move|_,w,cx|e.update(cx,|s,cx|{s.selected=row;s.run(w,cx)})));
 }}
 if self.rows.is_empty(){list=list.child("No commands found.");}
 div().id("command-palette").role(Role::Dialog).aria_label("Command palette").capture_key_down(cx.listener(Self::key)).size_full().bg(p.surface).text_color(p.text).p(px(24.)).flex().flex_col().gap(px(12.)).when(!self.open,|d|d.child("Command palette closed")).when(self.open,|d|d.child(TextField::new(&self.search,"Search commands")).when_some(self.controller.model.problem,|d,problem|d.child(div().role(Role::Alert).child(problem.message()))).child(list).child(div().text_color(p.muted).child("↑↓ navigate · Enter run · Esc close")))
 }
}
