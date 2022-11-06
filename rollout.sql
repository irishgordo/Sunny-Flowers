DROP TABLE IF EXISTS group_items;

CREATE TABLE group_items (
	id SERIAL PRIMARY KEY,
	url VARCHAR(255) NULL,
	name VARCHAR(255) NOT NULL,
	description text NULL,
	quantity integer default 1 not NULL,
    last_update DATE default now(),
    constraint quantity_nonnegative check (quantity >= 0)
);


INSERT INTO public.group_items
(url, "name", description, quantity)
VALUES('', 'magical-disk', 'a magical disk of sorts', 1);

DROP TABLE IF EXISTS timeline_events;

CREATE TABLE timeline_events (
    id SERIAL PRIMARY KEY,
    day integer default 0 not NULL,
    month VARCHAR(255) not null,
    event text not null,
    year varchar(255) default '1494 DR',
    last_update DATE default now(),
    logged_by VARCHAR(255) not null,
    CONSTRAINT chk_month CHECK (month IN ('Hammer', 'Alturiak', 'Ches', 'Tarsakh', 'Mirtul', 'Kythorn', 'Flamerule', 'Eleasis', 'Eleint', 'Marpenoth', 'Uktar', 'Nightal'))
);

insert into public.timeline_events (month, event, logged_by, year) VALUES('Hammer', 'its hammer time!', 'Odo', '1494 DR');
