//! Native-parity History workspace, hosted inside Root. No production I/O.
#[cfg(any(test,feature="gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{Button, ButtonIntent, ButtonSize, Choice, ChoiceKind, Choices, SelectionChanged, TextField, Toggle,
    data_components::Progress, field::{InputEvent, TextFieldState}, tokens::{self,Palette}};
use gpui::{prelude::*, *};
use host::{Authority,Host,Request,RequestKind};
use inbox::Inbox;
use model::*;
use std::{ops::Range,rc::Rc,time::Instant};

// The list callback only constructs visible rows. Detail pages independently
// bound shaping/AX to 2048 scalars; a megabyte transcript never becomes one label.
const DETAIL_PAGE:usize=2048;
const PREVIEW_CHARS:usize=160;
const ROW_HEIGHT:Pixels=px(108.);
pub struct History {
    pub model:Model,
    host:Rc<dyn Host>, inbox:Inbox, authority:Authority,
    subscription:Option<Box<dyn host::Subscription>>,
    subscriptions:Vec<gpui::Subscription>,
    pub search:Entity<TextFieldState>, source:Entity<Choices>,date:Entity<Choices>,actions:Entity<Choices>,
    focus:FocusHandle,filter_focus:FocusHandle,back_focus:FocusHandle,
    scroll:UniformListScrollHandle, filters_open:bool,detail_page:usize,
    pub rendered:Range<usize>,
    pub fixture:bool,
    notice_epoch:u64,
}
impl History {
    pub fn new(host:Rc<dyn Host>,w:&mut Window,cx:&mut Context<Self>)->Self {
        let search=cx.new(|cx|TextFieldState::new(w,cx).placeholder("Search"));
        let source=cx.new(|cx|Choices::new("Show transcripts",ChoiceKind::Dropdown,
            ["Everything","Spoken","Files"].iter().enumerate().map(|(i,l)|Choice::new(i.to_string(),*l)).collect(),w,cx));
        let date=cx.new(|cx|Choices::new("From date",ChoiceKind::Dropdown,
            ["Any time","Today","Past 7 days","Past 30 days"].iter().enumerate().map(|(i,l)|Choice::new(i.to_string(),*l)).collect(),w,cx));
        source.update(cx,|s,cx|s.set_value("0",cx));date.update(cx,|s,cx|s.set_value("0",cx));
        let actions=cx.new(|cx|Choices::new("More history actions",ChoiceKind::Dropdown,vec![],w,cx));
        let mut s=Self {model:Model::default(),host,inbox:Inbox::default(),authority:Authority::default(),subscription:None,subscriptions:vec![],
            search:search.clone(),source:source.clone(),date:date.clone(),actions:actions.clone(),focus:cx.focus_handle(),filter_focus:cx.focus_handle(),back_focus:cx.focus_handle(),
            scroll:UniformListScrollHandle::new(),filters_open:false,detail_page:0,rendered:0..0,fixture:false,notice_epoch:0};
        s.subscriptions.push(cx.subscribe(&search,|s,_,e:&InputEvent,cx| {if matches!(e,InputEvent::Change) {
            let mut q=s.model.query.clone();q.text=s.search.read(cx).value().to_string();s.query(q,cx);
        }}));
        s.subscriptions.push(cx.subscribe(&source,|s,_,e:&SelectionChanged,cx| {
            let mut q=s.model.query.clone();q.source=match e.0.as_ref() {"1"=>Source::Spoken,"2"=>Source::Files,_=>Source::All};s.query(q,cx);
        }));
        s.subscriptions.push(cx.subscribe(&date,|s,_,e:&SelectionChanged,cx| {
            let mut q=s.model.query.clone();q.date=match e.0.as_ref() {"1"=>Date::Today,"2"=>Date::Week,"3"=>Date::Month,_=>Date::All};s.query(q,cx);
        }));
        s.subscriptions.push(cx.subscribe_in(&actions,w,|s,_,e:&SelectionChanged,w,cx|s.action(&e.0,w,cx)));
        s
    }
    pub fn attach(&mut self,cx:&mut Context<Self>) {
        if self.subscription.is_some() {return;}
        self.model.attach();self.model.set_clock(local_clock());self.inbox=Inbox::default();self.authority.sync(&self.model);
        self.subscription=Some(self.host.subscribe(murmur_core::events::Consumer::Main,self.model.ticket().attachment,self.inbox.clone()));
        self.drain(cx);self.request(RequestKind::Read);self.prepare(cx);
    }
    pub fn detach(&mut self) {
        self.authority.revoke();self.inbox.close();if let Some(mut s)=self.subscription.take() {s.stop();}
        self.model.detach();self.filters_open=false;self.detail_page=0;self.rendered=0..0;
    }
    /// Clears editor search storage as well as transcript allocations on hide.
    pub fn hide(&mut self,w:&mut Window,cx:&mut Context<Self>) {
        self.detach();self.search.update(cx,|s,cx|s.set_value("",w,cx));cx.notify();
    }
    pub fn follow(&self,cx:&mut Context<Self>) {
        let attachment=self.model.ticket().attachment;
        cx.spawn(async move |weak,cx| {loop {
            cx.background_executor().timer(std::time::Duration::from_millis(32)).await;
            if !weak.update(cx,|s,cx| {
                if s.subscription.is_none()||s.model.ticket().attachment!=attachment||s.inbox.is_closed() {return false;}
                let old=s.model.ticket();s.model.set_clock(local_clock());
                let armed=s.model.confirmation.is_some();s.model.expire(Instant::now());
                s.drain(cx);s.authority.sync(&s.model);
                if old!=s.model.ticket()||armed!=s.model.confirmation.is_some() {cx.notify();}
                true
            }).unwrap_or(false) {break;}
        }}).detach();
    }
    pub fn receive(&mut self,e:Event,cx:&mut Context<Self>) {self.drain(cx);self.accept(e,cx);self.prepare(cx);}
    fn accept(&mut self,e:Event,cx:&mut Context<Self>)->bool {
        let accepted=self.model.event(e);self.authority.sync(&self.model);
        if accepted && self.model.notice.is_some() {
            self.notice_epoch+=1;let epoch=self.notice_epoch;
            cx.spawn(async move |weak,cx| {cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                let _=weak.update(cx,|s,cx|{if s.notice_epoch==epoch {s.model.notice=None;cx.notify();}});
            }).detach();
        }
        accepted
    }
    pub fn drain(&mut self,cx:&mut Context<Self>) {
        let mut retry=false;
        for e in self.inbox.drain() {
            let stale=matches!(&e,Event::Read{ticket,..} if ticket.attachment==self.model.ticket().attachment);
            if !self.accept(e,cx)&&stale {retry=true;}
        }
        if retry && self.subscription.is_some() {self.request(RequestKind::Read);}
        self.prepare(cx);
    }
    fn request(&mut self,kind:RequestKind) {
        if self.subscription.is_none() {return;}
        let ticket=self.model.begin();self.authority.sync(&self.model);
        self.host.request(Request{ticket,lease:self.authority.lease(ticket),kind},self.inbox.clone());
    }
    fn query(&mut self,q:Query,cx:&mut Context<Self>) {
        self.model.set_query(q);self.authority.sync(&self.model);self.scroll.scroll_to_item(0,ScrollStrategy::Top);self.prepare(cx);
    }
    pub fn prepare(&mut self,cx:&mut Context<Self>) {
        self.model.refresh();self.authority.sync(&self.model);
        let count=self.model.matches.len();
        let mut choices=vec![Choice::new("import","Transcribe audio or video file…")];
        for (save,verb) in [(false,"Copy"),(true,"Save")] {for (i,f) in Format::ALL.iter().enumerate() {
            choices.push(Choice::new(format!("{}-{i}",if save {"save"} else {"copy"}),format!("{verb} {count} shown as {}",f.label())).disabled(count==0));
        }}
        if self.model.query.narrowed() {choices.push(Choice::new("shown",format!("Delete {count} shown…")).disabled(count==0));}
        choices.push(Choice::new("clear","Clear History…").disabled(self.model.entries.is_empty()));
        self.actions.update(cx,|s,cx|s.replace_choices(choices,cx));cx.notify();
    }
    fn action(&mut self,value:&str,w:&mut Window,cx:&mut Context<Self>) {
        if value=="clear" {self.delete(DeleteScope::All,w,cx);}
        else if value=="shown" {self.delete(DeleteScope::Shown,w,cx);}
        else if value=="import" {self.request(RequestKind::TranscribeFile);}
        else if let Some((verb,index))=value.split_once('-') && let Ok(i)=index.parse::<usize>() && let Some(format)=Format::ALL.get(i) {
            let entries=self.model.matches.iter().map(|&i|self.model.entries[i].clone()).collect();
            self.request(RequestKind::Export{entries,format:*format,save:verb=="save"});
        }
        cx.notify();
    }
    fn delete(&mut self,scope:DeleteScope,w:&mut Window,cx:&mut Context<Self>) {
        if self.model.arm(scope.clone(),Instant::now()) {
            let Some(c)=self.model.confirmation.take() else {return;};
            let entries=self.model.entries.iter().filter(|e|c.keys.contains(&e.key)).cloned().collect();
            self.request(RequestKind::Delete{entries,clear_all:scope==DeleteScope::All});
            w.focus(&self.focus,cx);
        }
        cx.notify();
    }
    fn copy(&mut self,key:u64,markdown:bool) {
        if let Some(entry)=self.model.entries.iter().find(|e|e.key==key).cloned() {self.request(RequestKind::CopyRow{entry,markdown});}
    }
    fn open_detail(&mut self,key:u64,w:&mut Window,cx:&mut Context<Self>) {
        self.model.invalidate();self.authority.sync(&self.model);self.model.selected=Some(key);self.model.detail=Some(key);self.detail_page=0;
        w.focus(&self.back_focus,cx);cx.notify();
    }
    fn back(&mut self,w:&mut Window,cx:&mut Context<Self>) {
        self.model.invalidate();self.authority.sync(&self.model);self.model.detail=None;
        if let Some(i)=self.model.matches.iter().position(|&i|Some(self.model.entries[i].key)==self.model.selected) {self.scroll.scroll_to_item(i,ScrollStrategy::Nearest);}
        w.focus(&self.focus,cx);cx.notify();
    }
    fn reset(&mut self,w:&mut Window,cx:&mut Context<Self>) {
        self.search.update(cx,|s,cx|s.set_value("",w,cx));
        self.source.update(cx,|s,cx|s.set_value("0",cx));self.date.update(cx,|s,cx|s.set_value("0",cx));self.query(Query::default(),cx);
        w.focus(&self.search.read(cx).focus_handle(cx),cx);
    }
    fn key(&mut self,e:&KeyDownEvent,w:&mut Window,cx:&mut Context<Self>) {
        if e.prefer_character_input {return;}
        if e.keystroke.modifiers.platform && e.keystroke.key=="f" {
            w.focus(&self.search.read(cx).focus_handle(cx),cx);
        } else if e.keystroke.key=="escape" {
            if self.model.confirmation.take().is_some() {} else if self.model.detail.is_some() {self.back(w,cx);}
            else if self.filters_open {self.filters_open=false;w.focus(&self.filter_focus,cx);}
            else {self.search.update(cx,|s,cx|s.set_value("",w,cx));let mut q=self.model.query.clone();q.text.clear();self.query(q,cx);w.focus(&self.focus,cx);}
        } else {return;}
        w.prevent_default();cx.stop_propagation();cx.notify();
    }
    fn list_key(&mut self,e:&KeyDownEvent,w:&mut Window,cx:&mut Context<Self>) {
        if e.prefer_character_input||e.keystroke.modifiers.number_of_modifiers()!=0||self.model.matches.is_empty() {return;}
        let mut i=self.model.matches.iter().position(|&i|Some(self.model.entries[i].key)==self.model.selected).unwrap_or(0);
        match e.keystroke.key.as_str() {
            "down"=>i=(i+1).min(self.model.matches.len()-1),"up"=>i=i.saturating_sub(1),"home"=>i=0,"end"=>i=self.model.matches.len()-1,
            "pagedown"=>i=(i+5).min(self.model.matches.len()-1),"pageup"=>i=i.saturating_sub(5),
            "enter"|"space" if !e.is_held=>self.copy(self.model.entries[self.model.matches[i]].key,false),
            "right" if !e.is_held=>self.open_detail(self.model.entries[self.model.matches[i]].key,w,cx),
            _=>return,
        }
        self.model.selected=Some(self.model.entries[self.model.matches[i]].key);self.scroll.scroll_to_item(i,ScrollStrategy::Nearest);
        w.prevent_default();cx.stop_propagation();cx.notify();
    }
    fn rows(&mut self,range:Range<usize>,_:&mut Window,cx:&mut Context<Self>)->Vec<AnyElement> {
        self.rendered=range.clone();let p=Palette::get(cx);
        range.map(|position| {
            let entry=&self.model.entries[self.model.matches[position]];let key=entry.key;
            let label=format!("Transcript {} of {} from {}. Enter or Space copies; Right opens details.",position+1,self.model.matches.len(),stamp(entry.timestamp));
            let preview:String=entry.text.chars().take(PREVIEW_CHARS).collect();
            let text=highlighted(&preview,&self.model.query.text,p);
            div().id(("row",key)).h(ROW_HEIGHT).pb(tokens::LIST_GAP).child(
                div().id(("copy",key)).role(Role::Button).aria_label(label)
                    .aria_value(preview).aria_position_in_set(position+1).aria_set_size(self.model.matches.len())
                    .when(self.model.selected==Some(key),|d|d.aria_active_descendant().border_l_2().border_color(p.focus))
                    .h_full().overflow_hidden().p(px(8.)).rounded(tokens::CARD_RADIUS).bg(p.raised)
                    .flex().flex_col().gap(px(4.)).hover(|d|d.bg(p.surface))
                    .on_click(cx.listener(move |s,_,w,cx|{s.model.selected=Some(key);w.focus(&s.focus,cx);s.copy(key,false);cx.notify();}))
                    .on_a11y_action(AccessibleAction::Click,{let entity=cx.entity();move |_,w,cx|entity.update(cx,|s,cx|{s.model.selected=Some(key);w.focus(&s.focus,cx);s.copy(key,false);cx.notify();})})
                    .child(div().text_size(tokens::CAPTION_SIZE).text_color(p.muted).child(format!("{}{}{}{}",stamp(entry.timestamp),if entry.file {format!(" · {}",entry.source_label())} else {String::new()},if entry.pinned {" · Pinned"} else {""},if entry.interrupted {" · Interrupted · partial"} else {""})))
                    .child(text)
                    .child(Button::new(("detail",key),format!("Show more for transcript {}",position+1)).caption("Show more / actions").size(ButtonSize::ExtraSmall)
                        .on_click(cx.listener(move |s,_,w,cx|{cx.stop_propagation();s.open_detail(key,w,cx);})))
                .into_any_element()
        }).collect()
    }
    fn detail(&mut self,cx:&mut Context<Self>)->AnyElement {
        let Some(entry)=self.model.entries.iter().find(|e|Some(e.key)==self.model.detail).cloned() else {return div().into_any_element();};
        let key=entry.key;let pages=entry.text.chars().count().div_ceil(DETAIL_PAGE).max(1);self.detail_page=self.detail_page.min(pages-1);
        let page:String=entry.text.chars().skip(self.detail_page*DETAIL_PAGE).take(DETAIL_PAGE).collect();let p=Palette::get(cx);
        let mut view=div().id("detail").role(Role::Group).aria_label("Transcript detail").flex().flex_col().min_h_0().flex_1().gap(px(8.))
            .child(Button::new("back","Back to transcripts").track_focus(&self.back_focus).on_click(cx.listener(|s,_,w,cx|s.back(w,cx))))
            .child(stamp(entry.timestamp))
            .child(div().id("detail-text").role(Role::Label).aria_label(format!("Transcript text page {} of {pages}",self.detail_page+1)).aria_value(page.clone())
                .flex_1().min_h_0().overflow_y_scroll().child(highlighted(&page,&self.model.query.text,p)))
            .child(div().flex().flex_wrap().gap(px(8.))
                .child(Button::new("previous","Previous text page").disabled(self.detail_page==0).on_click(cx.listener(|s,_,_,cx|{s.detail_page=s.detail_page.saturating_sub(1);cx.notify();})))
                .child(format!("Page {} of {pages}",self.detail_page+1))
                .child(Button::new("next","Next text page").disabled(self.detail_page+1>=pages).on_click(cx.listener(move |s,_,_,cx|{s.detail_page=(s.detail_page+1).min(pages-1);cx.notify();}))))
            .child(div().flex().flex_wrap().gap(px(8.))
                .child(Button::new("copy-full","Copy full transcript").on_click(cx.listener(move |s,_,_,_|s.copy(key,false))))
                .child(Button::new("copy-md","Copy as Markdown").on_click(cx.listener(move |s,_,_,_|s.copy(key,true))))
                .child(Button::new("pin",if entry.pinned {"Unpin transcript"} else {"Pin transcript"})
                    .disabled(!entry.pinned && self.model.entries.iter().filter(|e|e.pinned).count()>=MAX_PINS)
                    .on_click(cx.listener(move |s,_,_,cx|{if let Some(entry)=s.model.entries.iter().find(|e|e.key==key).cloned() {s.request(RequestKind::Pin{entry});cx.notify();}})))
                .child(Button::new("delete-row","Delete transcript…").intent(ButtonIntent::Danger).on_click(cx.listener(move |s,_,w,cx|s.delete(DeleteScope::Row(key),w,cx)))));
        // Eligibility is the newest STORED entry, independent of sorting/filter.
        if self.model.entries.last().is_some_and(|e|e.key==key) {
            view=view.child(Button::new("teach","Correct & teach…").on_click(cx.listener(move |s,_,_,_| {
                if let Some(entry)=s.model.entries.last().filter(|e|e.key==key).cloned() {s.request(RequestKind::CorrectAndTeach{entry});}
            })));
        }
        view.into_any_element()
    }
}
impl Drop for History {fn drop(&mut self) {self.detach();}}
impl Render for History {
    fn render(&mut self,_:&mut Window,cx:&mut Context<Self>)->impl IntoElement {
        let p=Palette::get(cx);
        let mut root=div().id("history-root").role(Role::Group).aria_label("Transcription History").size_full().flex().flex_col().gap(px(8.)).p(px(16.))
            .bg(p.surface).text_color(p.text).font_family(".SystemUIFont").text_size(tokens::BODY_SIZE).line_height(tokens::BODY_LEADING)
            .capture_key_down(cx.listener(Self::key));
        if let Some(notice)=self.model.notice {root=root.child(div().id("notice").role(Role::Status).aria_label(notice).child(notice));}
        if let Some(c)=&self.model.confirmation {
            let scope=c.scope.clone();let label=format!("Confirm delete {} {}?",c.keys.len(),if c.keys.len()==1 {"transcript"} else {"transcripts"});
            root=root.child(div().id("confirm-delete").role(Role::Group).aria_label(label.clone()).flex().flex_wrap().gap(px(8.))
                .child(label).child("Confirmation expires in four seconds.")
                .child(Button::new("cancel-delete","Cancel deletion").on_click(cx.listener(|s,_,_,cx|{s.model.confirmation=None;cx.notify();})))
                .child(Button::new("confirm-delete-action","Confirm deletion").intent(ButtonIntent::Danger).on_click(cx.listener(move |s,_,w,cx|s.delete(scope.clone(),w,cx)))));
        }
        if self.model.retry_import {root=root.child(div().id("migration").role(Role::Status).child("Some older history is waiting for import. Retained entries remain available.")
            .child(Button::new("retry-import","Retry History import").on_click(cx.listener(|s,_,_,_|s.request(RequestKind::RetryImport)))));}
        if !self.model.retain_history && !self.model.loading {root=root.child(div().id("retention").role(Role::Status).child("Saving new transcripts is off. Existing history stays until you delete it."));}
        if let Some(problem)=self.model.problem {root=root.child(div().id("history-error").role(Role::Alert).child(match problem {Problem::Unavailable=>"History could not be refreshed. Try again.",Problem::Invalid=>"History data could not be displayed safely.",Problem::NewerSchema=>"This history was saved by a newer app. Restart with a compatible version."})
            .child(Button::new("retry","Retry History").on_click(cx.listener(|s,_,_,_|s.request(RequestKind::Read)))));}
        if self.model.detail.is_some() {return root.child(self.detail(cx));}
        root=root.child(div().flex().flex_wrap().items_center().gap(px(8.))
            .child("History").child(div().w(px(200.)).child(TextField::new(&self.search,"Search transcripts")))
            .child(Button::new("clear-search","Clear transcript search").size(ButtonSize::Compact).on_click(cx.listener(|s,_,w,cx|{
                s.search.update(cx,|s,cx|s.set_value("",w,cx));let mut q=s.model.query.clone();q.text.clear();s.query(q,cx);w.focus(&s.search.read(cx).focus_handle(cx),cx);
            })))
            .child(Button::new("filter",self.model.query.filter_label()).caption("Filter").size(ButtonSize::Compact).toggled(self.filters_open).track_focus(&self.filter_focus)
                .on_click(cx.listener(|s,_,_,cx|{s.filters_open=!s.filters_open;cx.notify();})))
            .child(self.actions.clone()));
        if self.filters_open {root=root.child(div().id("filters").role(Role::Group).aria_label("Transcript filters").flex().flex_wrap().gap(px(8.))
            .child(self.source.clone()).child(self.date.clone()).child("Pinned only")
            .child(Toggle::new("pinned","Pinned only",self.model.query.pinned).on_change(cx.listener(|s,pinned,_,cx|{let mut q=s.model.query.clone();q.pinned=*pinned;s.query(q,cx);})))
            .child(Button::new("clear-filters","Clear filters").on_click(cx.listener(|s,_,_,cx|{
                let mut q=s.model.query.clone();q.source=Source::All;q.date=Date::All;q.pinned=false;s.source.update(cx,|s,cx|s.set_value("0",cx));s.date.update(cx,|s,cx|s.set_value("0",cx));s.query(q,cx);
            }))));}
        if self.model.query.narrowed() {root=root.child(div().flex().items_center().gap(px(8.)).child(format!("Showing {} of {}",self.model.matches.len(),self.model.entries.len()))
            .child(Button::new("show-all","Show all").on_click(cx.listener(|s,_,w,cx|s.reset(w,cx)))));}
        if self.model.loading {return root.child(Progress::new("Loading History",None));}
        if self.model.matches.is_empty() {return root.child(div().role(Role::Label).child(if self.model.entries.is_empty() {"No transcription history yet. Your private, local transcripts will appear here."} else {"No matching transcripts"}));}
        root.child(div().id("history-list").role(Role::List).aria_label("Transcripts. Arrow keys navigate; Enter or Space copies; Right opens details.")
            .aria_set_size(self.model.matches.len()).track_focus(&self.focus).flex_1().min_h_0()
            .focus_visible(|d|d.border_1().border_color(p.focus)).capture_key_down(cx.listener(Self::list_key))
            .child(uniform_list("transcript-rows",self.model.matches.len(),cx.processor(Self::rows)).track_scroll(&self.scroll).size_full()))
    }
}
fn highlighted(text:&str,query:&str,p:Palette)->AnyElement {
    let highlights=highlights(text,query).into_iter().map(|range|(range,HighlightStyle {background_color:Some(p.accent.opacity(0.2)),color:Some(p.text),..Default::default()}));
    div().child(StyledText::new(text.to_owned()).with_highlights(highlights)).into_any_element()
}
pub fn stamp(ms:i64)->String {chrono::DateTime::from_timestamp_millis(ms).map(|d|d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(||"Unavailable time".into())}
pub fn local_clock()->Clock {
    use chrono::{Days,Local,TimeZone};let now=Local::now();let today=now.date_naive();
    let midnight=|days| today.checked_sub_days(Days::new(days)).and_then(|d|d.and_hms_opt(0,0,0)).and_then(|d|Local.from_local_datetime(&d).earliest()).map(|d|d.timestamp_millis()).unwrap_or(now.timestamp_millis());
    Clock{now:now.timestamp_millis(),today:midnight(0),week:midnight(6),month:midnight(29)}
}
